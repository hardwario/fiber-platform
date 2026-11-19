/* STM32WL55JC, the M4 core alone: 256 KB of flash, SRAM1 (32 KB) for this program. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 32K
}
