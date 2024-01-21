ENTRY(_kernel_entry)

.kernel_entry:
    mov si, kernel_hello_msg
    call print
    int3
    ret

kernel_hello_msg: db "kernel hello world",0