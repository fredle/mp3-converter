fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("FileDescription", "MP4 to MP3 & Audio Converter");
        res.set("ProductName", "MP4 to MP3 & Audio Converter");
        res.set("OriginalFilename", "mp3_converter.exe");
        if let Err(e) = res.compile() {
            // An icon is cosmetic - never fail the build over it.
            println!("cargo:warning=icon not embedded: {e}");
        }
    }
}
