//! IMG-005: a call written as the old `store(disk, path)` must not compile,
//! rather than store `out.png/<random>.png` on the default disk.

async fn store_the_old_way() {
    let _ = suprnova::Image::from_path("in.png")
        .store("images", "out.png")
        .await;
}

fn main() {
    let _ = store_the_old_way();
}
