/* Test infrastructure only: Multiboot 1 entry to x86-64 identity-mapped
 * long mode. The external bootstrap is not part of +A or a product kernel. */
.section .multiboot,"a"
.align 4
.long 0x1badb002
.long 0x00000003
.long -(0x1badb002 + 0x00000003)
.section .text,"ax"
.code32
.global boot_entry
boot_entry:
    cli
    movl $stack_top, %esp
    movl $pdpt, %eax
    orl $3, %eax
    movl %eax, pml4
    movl $pd, %eax
    orl $3, %eax
    movl %eax, pdpt
    xorl %ecx, %ecx
1:  movl %ecx, %eax
    shll $21, %eax
    orl $0x83, %eax
    movl %eax, pd(,%ecx,8)
    incl %ecx
    cmpl $512, %ecx
    jne 1b
    movl %cr4, %eax
    orl $0x20, %eax
    movl %eax, %cr4
    movl $pml4, %eax
    movl %eax, %cr3
    movl $0xc0000080, %ecx
    rdmsr
    orl $0x100, %eax
    wrmsr
    lgdt gdt_ptr
    movl %cr0, %eax
    orl $0x80000001, %eax
    movl %eax, %cr0
    ljmp $0x08, $long_mode
.code64
long_mode:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl $stack_top, %esp
    movabsq $0x200000, %rax
    jmp *%rax
.align 8
gdt:
    .quad 0
    .quad 0x00af9a000000ffff
    .quad 0x00cf92000000ffff
gdt_end:
gdt_ptr:
    .word gdt_end-gdt-1
    .long gdt
.section .bss,"aw",@nobits
.align 4096
pml4: .skip 4096
pdpt: .skip 4096
pd: .skip 4096
.align 16
.skip 16384
stack_top:
.section .note.GNU-stack,"",@progbits
