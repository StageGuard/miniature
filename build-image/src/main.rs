use std::{io::{Result, self, Seek}, env::{self}, path::Path, fs::{self}};
use tempfile::NamedTempFile;

const FILE_UEFI_BOOT: &str = "EFI/BOOT/BOOTX64.EFI";
const FILE_KERNEL: &str = "kernel-x86_64";
const MB: u64 = 1024 * 1024;

fn main() -> Result<()> {
    let DEFAULT_FILES_MAPPING: String = format!("{FILE_UEFI_BOOT}->target/x86_64-unknown-uefi/debug/bootloader.efi;{FILE_KERNEL}->target/x86_64-unknown-none/debug/kernel");
    let DEFAULT_OUTPUT_PATH: String = format!("{}", "target/os.img");

    let args: Vec<String> = env::args().collect();

    let env_files_mapping = args.get(1).unwrap_or(&DEFAULT_FILES_MAPPING);
    let output_path = args.get(2).unwrap_or(&DEFAULT_OUTPUT_PATH);

    let mut files_mapping: Vec<(&str, &str)> = Vec::new();
    env_files_mapping.split(";").for_each(|entry| {
        let paths: Vec<&str> = entry.trim().split("->").collect();
        files_mapping.push((paths.get(0).unwrap().trim(), paths.get(1).unwrap().trim()));
    });


    let fs_img = construct_filesystem_fat(&files_mapping)?;
    create_gpt_disk(fs_img.path(), Path::new(&output_path))?;
    fs_img.close()?;
    
    Ok(())
}

pub fn construct_filesystem_fat(
    files: &Vec<(&str, &str)>,
) -> Result<NamedTempFile> {

    let out_file = NamedTempFile::new()?;
    let out_file_path = out_file.path();

    // calculate needed size
    let mut needed_size = 0;
    for (_, src) in files {
        needed_size += fs::metadata(*src)?.len();
    }

    let fat_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_file_path)
        .unwrap();
    let fat_size_padded_and_rounded = ((needed_size + 1024 * 64 - 1) / MB + 1) * MB + MB;
    fat_file.set_len(fat_size_padded_and_rounded).unwrap();

    let label = *b"___ASOS____";

    let format_options = fatfs::FormatVolumeOptions::new().volume_label(label);
    fatfs::format_volume(&fat_file, format_options)?;
    let filesystem = fatfs::FileSystem::new(&fat_file, fatfs::FsOptions::new())?;
    let root_dir = filesystem.root_dir();

    for (dst, src) in files {
        let target_path = Path::new(*dst);

        let ancestors: Vec<_> = target_path.ancestors().skip(1).collect();
        for ancestor in ancestors.into_iter().rev().skip(1) {
            root_dir.create_dir(&ancestor.display().to_string())?;
        }

        let mut new_file = root_dir.create_file(*dst)?;
        new_file.truncate().unwrap();

        println!("copying {} to fs image: {}", *src, *dst);
        io::copy(&mut fs::File::open(*src)?, &mut new_file)?;
    }

    println!("fat filesystem temp image is created at {}", out_file_path.display());
    Ok(out_file)
}

pub fn create_gpt_disk(fat_image: &Path, out_image_path: &Path) -> Result<()> {
    let mut disk = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(out_image_path)?;

    let partition_size: u64 = fs::metadata(fat_image)?.len();
    let disk_size = partition_size + 1024 * 64; // for GPT headers
    disk.set_len(disk_size)?;

    let mbr = gpt::mbr::ProtectiveMBR::with_lb_size(
        u32::try_from((disk_size / 512) - 1).unwrap_or(0xFF_FF_FF_FF),
    );
    mbr.overwrite_lba0(&mut disk)?;

    let block_size = gpt::disk::LogicalBlockSize::Lb512;
    let mut gpt = gpt::GptConfig::new()
        .writable(true)
        .initialized(false)
        .logical_block_size(block_size)
        .create_from_device(Box::new(&mut disk), None)?;
    gpt.update_partitions(Default::default())?;

    let partition_id = gpt
        .add_partition("boot", partition_size, gpt::partition_types::EFI, 0, None)?;
    let partition = gpt.partitions().get(&partition_id).unwrap();
    let start_offset = partition.bytes_start(block_size)?;

    gpt.write()?;

    disk.seek(io::SeekFrom::Start(start_offset))?;
    io::copy(&mut fs::File::open(fat_image)?, &mut disk)?;

    println!("gpt partition disk image is created at {}", out_image_path.display());
    Ok(())
}
