use uefi::{table::{SystemTable, Boot, boot::SearchType}, proto::console::gop::{GraphicsOutput, PixelFormat}, Identify};
use shared::{framebuffer::{FBPixelFormat, Framebuffer}, print_panic::PrintPanic};


pub fn locate_framebuffer(system_table: &SystemTable<Boot>) -> Option<Framebuffer> {
    let boot_services = system_table.boot_services();

    let graphics_output_handle_buffer = match boot_services
        .locate_handle_buffer(SearchType::ByProtocol(&GraphicsOutput::GUID))
    {
        Ok(handle_buffer) => handle_buffer,
        Err(e) => {
            return None
        }
    };

    let graphics_output_handle = match graphics_output_handle_buffer.first() {
        Some(handle) => *handle,
        None => {
            return None;
        },
    };

    let mut protocol = match boot_services.open_protocol_exclusive::<GraphicsOutput>(graphics_output_handle) {
        Ok(p) => p,
        Err(e) => {
            return None
        }
    };

    let largest_resolution_mode = protocol
        .modes(boot_services)
        .filter(|mode| {
            let (width, height) = mode.info().resolution();
            width <= 1600 && height <= 900 
        })
        .max_by(|a, b| {
            let (a_width, a_height) = a.info().resolution();
            let (b_width, b_height) = b.info().resolution();

            (a_width * a_height).cmp(&(b_width * b_height))
        });
        
    if let Some(mode) = largest_resolution_mode {
        protocol.set_mode(&mode)
            .or_panic("failed to set graphics output mode");
    }

    let current_info = protocol.current_mode_info();
    let mut framebuffer = protocol.frame_buffer();

    Some(Framebuffer::new(
        framebuffer.as_mut_ptr(), 
        framebuffer.size(), 
        current_info.resolution().0, 
        current_info.resolution().1, 
        current_info.stride(), 
        match current_info.pixel_format() {
            PixelFormat::Rgb => FBPixelFormat::RGB,
            PixelFormat::Bgr => FBPixelFormat::BGR,
            others => return None
        }
    ))
}