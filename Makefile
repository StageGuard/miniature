QEMU_EXECUTABLE := qemu-system-x86_64
TARGET := x86_64-unknown-uefi
MODE := debug

CODE_FD_EXTERNAL = "C:\\Program Files\\qemu\share\\edk2-x86_64-code.fd"

setup:
	@mkdir -p test_esp/efi/boot
	@cp $(CODE_FD) test_esp/efi/boot/code.fd
	@cp target/$(TARGET)/$(MODE)/asos.efi test_esp/efi/boot/bootx64.efi

run: setup
	$(QEMU_EXECUTABLE) -enable-kvm \
        -drive if=pflash,format=raw,readonly=on,file=test_esp/efi/boot/code.fd \
        -drive format=raw,file=fat:rw:test_esp