/* nice!nano v2 (nRF52840) application region, same as the eyelash_corne
   partitions: Adafruit UF2 bootloader with its SoftDevice area below 0x26000,
   settings storage at 0xEC000, bootloader at 0xF4000. */
MEMORY
{
  FLASH : ORIGIN = 0x00026000, LENGTH = 792K
  RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}
