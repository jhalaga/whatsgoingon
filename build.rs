fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "WhatsGoingOn (wgo)");
        res.set("FileDescription", "Windows Terminal & Shell Monitor");
        res.compile()
            .expect("failed to embed Windows icon resource");
    }
}
