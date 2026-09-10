//! Embed the app icon and version info (#26). winresource writes the
//! resource object itself — no external rc.exe in the toolchain (the
//! Options dialog's no-rc decision was about dialog TEMPLATES; this is
//! icon/version only, README Differences).

fn main() {
    println!("cargo:rerun-if-changed=res/riviv.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("res/riviv.ico");
    // The icon lands as the FIRST icon resource — id 1 — which the window
    // class loads via LoadImageW(MAKEINTRESOURCEW(1)) (upstream
    // viv.c:5346-5350 loads its rc icon the same way).
    res.set("FileDescription", "riviv — image viewer");
    res.set("ProductName", "riviv");
    res.set("ProductVersion", "0.1.0");
    res.set("FileVersion", "0.1.0.0");
    res.set(
        "LegalCopyright",
        "MIT License — original C implementation © voidtools / David Carpenter",
    );
    if let Err(e) = res.compile() {
        // Fail the build (ADR 0001): a release exe silently missing its
        // icon/version metadata would ship inside installers undetected.
        panic!("winresource failed: {e}");
    }
}
