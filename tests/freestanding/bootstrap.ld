OUTPUT_FORMAT(elf32-i386)
ENTRY(boot_entry)
SECTIONS {
  . = 0x100000;
  .multiboot : { KEEP(*(.multiboot)) }
  .text : { *(.text) }
  .bss : { *(.bss) }
  . = 0x200000;
  .kernel : { *(.kernel) }
}
