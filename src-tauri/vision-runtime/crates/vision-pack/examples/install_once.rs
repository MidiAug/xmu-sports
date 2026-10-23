use std::env;
use std::path::PathBuf;
use vision_pack::install_from_staging;

fn main() {
    let data = PathBuf::from(env::var("XMU_SPORTS_DATA").expect("XMU_SPORTS_DATA"));
    let staging = data.join("staging");
    let st = install_from_staging(&data, &staging, "x86_64-pc-windows-msvc", 1).expect("install");
    println!("installed status={:?} version={:?}", st.status, st.active_version);
}
