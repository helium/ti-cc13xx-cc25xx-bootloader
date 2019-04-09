extern crate cc131x;
extern crate crc;
extern crate sysfs_gpio;

mod tests {
    use cc131x::firmware_image::FirmwareImage;
    use cc131x::Cc131x;

    #[test]
    fn test_startup() {
        let io = Cc131x::new("/dev/spidev2.1", 71, 72, 73, 74).unwrap();

        const FW_FILE1: &'static str = include_str!("../src/firmware/test_parsing.ihex");
        let firmware1 = FirmwareImage::new(FW_FILE1);
        let need_to_update_firmware = io.need_to_update_firmware(&firmware1).unwrap();
        if need_to_update_firmware {
            io.flash_firmware(&firmware1).unwrap();
        }

        const FW_FILE2: &'static str =
            include_str!("../firmware/gateway_CC1310_LAUNCHXL_tirtos_gcc.hex");
        let firmware2 = FirmwareImage::new(FW_FILE2);
        let need_to_update_firmware = io.need_to_update_firmware(&firmware2).unwrap();
        if need_to_update_firmware {
            io.flash_firmware(&firmware2).unwrap();
        }
    }
}
