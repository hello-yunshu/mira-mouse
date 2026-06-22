// SPDX-License-Identifier: AGPL-3.0-or-later
// List all HID devices and their interfaces. Useful for debugging HID issues.
use hidapi::HidApi;

fn main() {
    let api = HidApi::new().expect("initialize HID API");
    println!("HID devices:");
    for device in api.device_list() {
        let path = device.path().to_string_lossy();
        let manufacturer = device.manufacturer_string().unwrap_or("?");
        let product = device.product_string().unwrap_or("?");
        let interface = device.interface_number();
        println!(
            "  vid={:04x} pid={:04x} usage_page=0x{:04x} usage=0x{:04x} interface={} path={}",
            device.vendor_id(),
            device.product_id(),
            device.usage_page(),
            device.usage(),
            interface,
            path,
        );
        println!("    manufacturer={:?} product={:?}", manufacturer, product);
    }
}
