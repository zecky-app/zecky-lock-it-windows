//! Asks Windows which kinds of key it offers, and tries to set up each one.
//! On a PC without Windows Hello or a security key the answer must be a
//! calm "not available", never a crash.

fn main() {
    #[cfg(windows)]
    {
        use lockit_auth::{
            keys::{security_key::SecurityKey, windows_hello::WindowsHelloKey},
            KeySource,
        };
        println!("hello available: {}", WindowsHelloKey::is_available());
        println!("security key available: {}", SecurityKey::is_available());
        println!("security key API version: {:?}", SecurityKey::api_version());
        match WindowsHelloKey::new().enroll("This PC") {
            Ok(_) => println!("hello enroll: ok"),
            Err(e) => println!("hello enroll: {e}"),
        }
        match SecurityKey::new().enroll("Security key") {
            Ok(_) => println!("security key enroll: ok"),
            Err(e) => println!("security key enroll: {e}"),
        }
    }
    #[cfg(not(windows))]
    println!("Windows only");
}
