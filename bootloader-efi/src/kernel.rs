use xmas_elf::ElfFile;


pub struct Kernel<'a> {
    pub elf: ElfFile<'a>,
    pub start_address: *const u8,
    pub len: usize,
}

impl<'a> Kernel<'a> {
    pub fn parse(kernel_slice: &'a [u8]) -> Self {
        let kernel_elf = ElfFile::new(kernel_slice).unwrap();
        Kernel {
            elf: kernel_elf,
            start_address: kernel_slice.as_ptr(),
            len: kernel_slice.len(),
        }
    }
}