fn main() {
    println!("cargo:rerun-if-env-changed=VALOFRAME_AD_MANIFEST_ENDPOINT");
    println!("cargo:rerun-if-env-changed=VALOFRAME_AD_ALLOWED_HOSTS");
    tauri_build::build()
}
