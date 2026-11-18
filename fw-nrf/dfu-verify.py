#!/usr/bin/env python3
"""Check signed DFU packages and the signed bootloader without hardware.

    dfu-verify.py pubkey  --key-file PEM --elf BOOTLOADER.elf --uf2 update-....uf2 [--hex ..._nosd.hex]
    dfu-verify.py package ZIP (--key-file PEM | --elf BOOTLOADER.elf --uf2 update-....uf2) [--app-hex APP.hex]

`pubkey`: the 64-byte public key the bootloader verifies with (symbol `pk` in
the ELF, read back from the UF2/hex that actually gets flashed) is the public
half of the key file.

`package`: replays what Adafruit nRF52 bootloader 0.11.0 with SIGNED_FW does
with a serial DFU package (src/dfu_init.c, lib/sdk11/.../dfu_transport_serial.c
and dfu_single_bank.c at tag 0.11.0):
  - start packet: app size a multiple of 4 and <= DFU_IMAGE_MAX_SIZE_FULL;
  - the init packet as it arrives over serial: nrfutil appends 2 zero bytes,
    the transport keeps whole words only;
  - dfu_init_prevalidate: device type 0x0052, an --sd-req entry matching the
    installed SoftDevice (S140 6.1.1 = FWID 0x00B6), extension id 2 (ECDS),
    then ECDSA P-256 over SHA-256 of the init packet up to and including the
    image length and hash (r||s, 64 bytes), as tinycrypt's uECC_verify;
  - dfu_init_postvalidate: image length and SHA-256 equal the signed values.
It also checks the layout this project needs: the image starts at 0x26000
and stays below the bond storage at 0xE4000.

Needs `cryptography` and `ecdsa` (the venv from docs/signed-bootloader.md). Exit 1 on any
failure. Treats its inputs as data only.
"""

import argparse
import hashlib
import json
import math
import struct
import sys
import zipfile
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import (
    Prehashed,
    encode_dss_signature,
)

ADAFRUIT_DEVICE_TYPE = 0x0052
S140_6_1_1_FWID = 0x00B6  # read from the SoftDevice hex: info struct at 0x3000, offset 0x0C
DFU_SOFTDEVICE_ANY = 0xFFFE
EXT_USES_ECDS = 2
EXT_LENGTH_SIGNED = 40  # id (4) + image length (4) + SHA-256 (32)
EXT_LENGTH_MAX = 104  # m_extended_packet[] in dfu_init.c
INIT_PACKET_MAX = 128  # m_init_packet[] in dfu_single_bank.c
APP_START = 0x26000  # CODE_REGION_1_START = S140 6.1.1 size (fw-nrf/memory.x)
BOOTLOADER_START = 0xF4000
DFU_APP_DATA_RESERVED = 10 * 4096  # Makefile, nrf52840
DFU_IMAGE_MAX_SIZE_FULL = BOOTLOADER_START - APP_START - DFU_APP_DATA_RESERVED
DFU_BL_IMAGE_MAX_SIZE = 0xFE000 - BOOTLOADER_START  # up to the MBR params page
ADAFRUIT_DEV_REV = 52840  # dfu_init.c: needed for SoftDevice/bootloader updates
STORAGE_START = 0xE4000  # bonds (fw-nrf/memory.x STORAGE)
SELF_UPDATE_STAGING = 0xA3000  # 0.6.0 stages a new bootloader at 0xA3000..0xAD000
P256_N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551

failures: list[str] = []


def check(ok: bool, what: str) -> bool:
    print(("  ok    " if ok else "  FAIL  ") + what)
    if not ok:
        failures.append(what)
    return ok


def input_file(path: str) -> Path:
    """The file named on the command line, resolved; exits if it isn't an existing regular file."""
    resolved = Path(path).resolve()
    if not resolved.is_file():
        raise SystemExit(f"{path}: no such file")
    return resolved


def pubkey_from_pem(path: str) -> bytes:
    # The key is read with python-ecdsa, as adafruit-nrfutil does (it writes
    # PKCS#8 v2, which `cryptography` refuses); verifying below uses OpenSSL
    # through `cryptography`, so signing and checking are separate code.
    from ecdsa import SigningKey  # noqa: PLC0415 - only needed with --key-file

    return SigningKey.from_pem(input_file(path).read_bytes()).get_verifying_key().to_string()


def elf_symbol(path: str, name: str) -> tuple[int, int]:
    """Address and size of a symbol in a little-endian ELF32 file."""
    data = input_file(path).read_bytes()
    if data[:4] != b"\x7fELF" or data[4] != 1 or data[5] != 1:
        raise SystemExit(f"{path}: not a little-endian ELF32 file")
    shoff, = struct.unpack_from("<I", data, 0x20)
    shentsize, shnum = struct.unpack_from("<HH", data, 0x2E)
    sections = [struct.unpack_from("<10I", data, shoff + i * shentsize) for i in range(shnum)]
    for sh in sections:
        if sh[1] != 2:  # SHT_SYMTAB
            continue
        strtab = sections[sh[6]]
        for off in range(sh[4], sh[4] + sh[5], sh[9]):
            st_name, st_value, st_size = struct.unpack_from("<III", data, off)
            end = data.index(b"\0", strtab[4] + st_name)
            if data[strtab[4] + st_name:end].decode() == name:
                return st_value, st_size
    raise SystemExit(f"{path}: no symbol {name}")


def uf2_memory(path: str) -> dict[int, int]:
    mem: dict[int, int] = {}
    data = input_file(path).read_bytes()
    for i in range(0, len(data), 512):
        m0, m1, _flags, addr, size = struct.unpack_from("<5I", data, i)
        if (m0, m1) != (0x0A324655, 0x9E5D5157):
            raise SystemExit(f"{path}: bad UF2 block at {i}")
        for j in range(size):
            mem[addr + j] = data[i + 32 + j]
    return mem


def hex_memory(path: str) -> dict[int, int]:
    mem: dict[int, int] = {}
    upper = 0
    for raw_line in input_file(path).read_text().splitlines():
        line = raw_line.strip()
        if not line.startswith(":"):
            continue
        raw = bytes.fromhex(line[1:])
        n, addr, kind = raw[0], int.from_bytes(raw[1:3], "big"), raw[3]
        if kind == 0:
            for j in range(n):
                mem[upper + addr + j] = raw[4 + j]
        elif kind == 4:
            upper = int.from_bytes(raw[4:6], "big") << 16
        elif kind == 2:
            upper = int.from_bytes(raw[4:6], "big") << 4
    return mem


def read(mem: dict[int, int], addr: int, size: int) -> bytes | None:
    if any(a not in mem for a in range(addr, addr + size)):
        return None
    return bytes(mem[a] for a in range(addr, addr + size))


def bootloader_pubkey(elf: str, uf2: str, hexfile: str | None = None) -> bytes | None:
    addr, size = elf_symbol(elf, "pk")
    check(size == 64, f"bootloader symbol pk at 0x{addr:05X}, {size} bytes")
    pk = read(uf2_memory(uf2), addr, 64)
    check(pk is not None, f"pk is inside the self-update UF2 ({uf2})")
    if hexfile:
        check(read(hex_memory(hexfile), addr, 64) == pk, f"same pk in {hexfile}")
    return pk


def ecdsa_ok(pk: bytes, digest: bytes, sig: bytes) -> bool:
    r, s = int.from_bytes(sig[:32], "big"), int.from_bytes(sig[32:], "big")
    if not (0 < r < P256_N and 0 < s < P256_N):
        return False
    try:
        key = ec.EllipticCurvePublicNumbers(
            int.from_bytes(pk[:32], "big"), int.from_bytes(pk[32:], "big"), ec.SECP256R1()
        ).public_key()  # raises if the point is not on the curve (uECC_valid_public_key)
        key.verify(encode_dss_signature(r, s), digest, ec.ECDSA(Prehashed(hashes.SHA256())))
        return True
    except (InvalidSignature, ValueError):
        return False


def verify_package(path: str, pk: bytes, app_hex: str | None = None) -> bool:
    """True if the bootloader would accept the package. Its own checks are
    marked as such; the others are this project's rules (layout, sd-req)."""
    refused: list[str] = []

    def bl(ok: bool, what: str) -> bool:
        if not check(ok, "bootloader: " + what):
            refused.append(what)
        return ok

    with zipfile.ZipFile(input_file(path)) as z:
        manifest = json.loads(z.read("manifest.json"))["manifest"]
        parts = sorted(k for k in manifest if k != "dfu_version")
        check(parts in (["application"], ["bootloader"]), f"package holds an application or a bootloader: {parts}")
        dfu_version = manifest.get("dfu_version")
        signed = isinstance(dfu_version, float) and math.isclose(dfu_version, 0.8)
        check(signed, f"dfu_version {dfu_version} (0.8 = signed)")
        kind = parts[0]
        app = manifest[kind]
        dat = z.read(app["dat_file"])
        image = z.read(app["bin_file"])

    # Start packet (dfu_start_pkt_handle). Note: the bootloader erases the
    # application area for this size right here, before any signature check.
    bl(len(image) % 4 == 0, f"image size {len(image)} is word sized")
    bl(len(image) <= DFU_IMAGE_MAX_SIZE_FULL, f"image fits ({len(image)} <= {DFU_IMAGE_MAX_SIZE_FULL})")
    if kind == "bootloader":
        bl(len(image) <= DFU_BL_IMAGE_MAX_SIZE, f"bootloader fits ({len(image)} <= {DFU_BL_IMAGE_MAX_SIZE})")

    # Init packet as the bootloader receives it.
    sent = dat + b"\0\0"
    init = sent[: (len(sent) // 4) * 4]
    bl(len(init) <= INIT_PACKET_MAX, f"init packet {len(init)} bytes fits m_init_packet")
    device_type, device_rev, app_version, sd_len = struct.unpack_from("<HHIH", init, 0)
    sd = list(struct.unpack_from(f"<{sd_len}H", init, 10))
    ext = init[10 + 2 * sd_len:]
    print(f"  info  init packet: {len(dat)} bytes (.dat), {len(init)} bytes after the serial transport; "
          f"dev-type 0x{device_type:04X} dev-rev 0x{device_rev:04X} app-version 0x{app_version:08X} "
          f"sd-req {[hex(x) for x in sd]}")
    bl(device_type == ADAFRUIT_DEVICE_TYPE, "device type 0x0052")
    if kind == "bootloader":
        bl(device_rev == ADAFRUIT_DEV_REV, f"device revision {ADAFRUIT_DEV_REV} (required for a bootloader update)")
    bl(S140_6_1_1_FWID in sd or DFU_SOFTDEVICE_ANY in sd, "sd-req accepts S140 6.1.1 (0x00B6)")
    check(sd == [S140_6_1_1_FWID], "sd-req is exactly [0x00B6] (no 0xFFFE wildcard; one entry keeps the "
          "extension at 104 bytes)")
    check(len(ext) == EXT_LENGTH_MAX, f"extension is {len(ext)} bytes (m_extended_packet holds {EXT_LENGTH_MAX})")
    if not bl(len(ext) >= 4 and struct.unpack_from("<I", ext, 0)[0] == EXT_USES_ECDS,
              "extension id 2 (ECDS): the init packet is signed"):
        return False
    if not bl(len(ext) >= EXT_LENGTH_MAX, "extension long enough for hash and signature"):
        return False
    image_len, = struct.unpack_from("<I", ext, 4)

    # dfu_init_prevalidate: signature over the init packet minus r||s.
    msg = init[: len(init) - len(ext) + EXT_LENGTH_SIGNED]
    bl(ecdsa_ok(pk, hashlib.sha256(msg).digest(), ext[40:104]),
       "ECDSA P-256 signature over the init packet verifies with the bootloader key")

    # dfu_init_postvalidate.
    bl(image_len == len(image), f"signed image length {image_len} == {len(image)}")
    bl(ext[8:40] == hashlib.sha256(image).digest(), "signed SHA-256 == SHA-256 of the image")

    # Where the bootloader writes it: DFU_BANK_0_REGION_START = 0x26000.
    if app_hex and kind == "application":
        mem = hex_memory(app_hex)
        lo, hi = min(mem), max(mem) + 1
        check(lo == APP_START, f"{app_hex} starts at 0x{lo:05X} (must be 0x{APP_START:05X})")
        check(hi <= STORAGE_START, f"ends at 0x{hi:05X}, below the bond storage at 0x{STORAGE_START:05X}")
        check(hi <= SELF_UPDATE_STAGING, f"ends below 0x{SELF_UPDATE_STAGING:05X} (0.6.0 bootloader self-update "
              "staging area)")
        padded = bytes(mem.get(a, 0xFF) for a in range(lo, lo + len(image)))
        check(padded == image, "package image == the .hex contents")
    return not refused


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("pubkey")
    p.add_argument("--key-file", required=True)
    p.add_argument("--elf", required=True)
    p.add_argument("--uf2", required=True)
    p.add_argument("--hex")
    p = sub.add_parser("package")
    p.add_argument("zip")
    p.add_argument("--key-file")
    p.add_argument("--elf")
    p.add_argument("--uf2")
    p.add_argument("--app-hex")
    p.add_argument(
        "--expect-reject",
        action="store_true",
        help="succeed only if the bootloader would refuse the package",
    )
    a = ap.parse_args()

    if a.cmd == "pubkey":
        print(f"bootloader public key vs {a.key_file}:")
        pk = bootloader_pubkey(a.elf, a.uf2, a.hex)
        want = pubkey_from_pem(a.key_file)
        print(f"  info  Qx {want[:32].hex()}\n  info  Qy {want[32:].hex()}")
        check(pk == want, "the key compiled into the bootloader is the key file's public key")
    else:
        if a.elf and a.uf2:
            pk = bootloader_pubkey(a.elf, a.uf2)
            source = a.uf2
        elif a.key_file:
            pk = pubkey_from_pem(a.key_file)
            source = a.key_file
        else:
            ap.error("package needs --key-file or --elf with --uf2")
        print(f"{a.zip} against the key in {source}:")
        accepted = verify_package(a.zip, pk, a.app_hex)
        if a.expect_reject:
            # The bootloader refusing is the point; the other checks still count.
            failures[:] = [f for f in failures if not f.startswith("bootloader: ")]
            check(not accepted, "the bootloader would refuse this package, as expected")
        else:
            print(f"  info  the bootloader would {'accept' if accepted else 'REFUSE'} this package")
    if failures:
        print(f"{len(failures)} check(s) failed")
        sys.exit(1)
    print("all checks passed")


if __name__ == "__main__":
    main()
