/* The STM32WL55JC: 256 KB of flash, 64 KB of RAM (SRAM1 and SRAM2, one after the other). The last
   two pages of the flash (4 KB from 0x0803F000) keep the transmit counter's watermark
   (src/watermark.rs), so the image may not reach them. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 252K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
