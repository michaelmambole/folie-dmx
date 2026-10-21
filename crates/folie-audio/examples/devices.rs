//! Liste les entrées audio : `cargo run -p folie-audio --example devices`.

fn main() {
    for device in folie_audio::input::input_devices() {
        println!("{} ({} canaux)", device.name, device.channels);
    }
}
