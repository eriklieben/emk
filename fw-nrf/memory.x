/* nice!nano v2 (nRF52840) application region, same as the eyelash_corne
   partitions: Adafruit UF2 bootloader with its SoftDevice area below 0x26000,
   ZMK's settings storage at 0xEC000, bootloader at 0xF4000.

   STORAGE is this firmware's own flash for Bluetooth bonds: the last 32K of
   the application region, just below ZMK's settings, so switching between
   this firmware and ZMK keeps both sets of pairings. */
MEMORY
{
  FLASH   : ORIGIN = 0x00026000, LENGTH = 760K
  STORAGE : ORIGIN = 0x000E4000, LENGTH = 32K
  RAM     : ORIGIN = 0x20000000, LENGTH = 256K
}

__storage_start = ORIGIN(STORAGE);
__storage_end = ORIGIN(STORAGE) + LENGTH(STORAGE);
