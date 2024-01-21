cp target/x86_64-unknown-uefi/debug/asos.efi test_esp/efi/boot/bootx64.efi
qemu-system-x86_64 \
  -drive if=pflash,format=raw,readonly=on,file="/c/Program Files/qemu/share/edk2-x86_64-code.fd" \
  -drive format=raw,file=fat:rw:test_esp