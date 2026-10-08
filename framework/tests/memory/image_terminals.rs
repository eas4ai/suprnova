//! MEM-003 on the image terminals that hand back encoded output: each one
//! allocates what running the pipeline allocates, plus the one buffer its
//! own result needs, never a second full copy of that result, and returns
//! what `to_bytes` returns.

#![cfg(feature = "media")]

use std::future::Future;

use base64::Engine as _;
use oxideav_png::PngPixelFormat;
use suprnova::{Image, ImageDriverKind};

use crate::support::{Heap, exclusive};

/// The side of the square source: its BMP is 4 MiB and its base64 over
/// 5 MiB, so a second full copy of either is far above everything else that
/// varies.
const SIDE: u32 = 1024;

/// A `SIDE` by `SIDE` PNG of one colour.
fn source() -> bytes::Bytes {
    let stride = SIDE as usize * 3;
    let png = oxideav_png::encode_plane(
        SIDE,
        SIDE,
        PngPixelFormat::Rgb24,
        stride,
        &[0x20, 0x80, 0xE0].repeat(SIDE as usize * SIDE as usize),
        None,
        &oxideav_png::EncodeOptions::default(),
    )
    .expect("the PNG encodes");
    bytes::Bytes::from(png)
}

/// The source converted to a bitmap on the built-in driver, so the output's
/// size does not depend on what an encoder can compress.
fn bitmap(source: &bytes::Bytes) -> Image {
    Image::from_bytes(source.clone())
        .using(ImageDriverKind::OxideAv)
        .to_bmp()
}

/// The bytes everything allocated while `op` ran.
async fn allocated<T>(heap: &Heap, op: impl Future<Output = T>) -> (T, u64) {
    let before = heap.bytes();
    let out = op.await;
    (out, heap.bytes() - before)
}

/// MEM-003: a data URI is the prefix and the base64 written into one
/// buffer, not the base64 written once and copied after the prefix.
#[tokio::test]
async fn mem_audit_an_image_data_uri_is_written_in_one_buffer() {
    let _lock = exclusive().await;
    let source = source();
    let output = bitmap(&source).to_bytes().await.expect("a warm-up");
    let mime = bitmap(&source).mime_type().await.expect("the media type");
    let expected = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&output)
    );
    bitmap(&source).to_data_uri().await.expect("a warm-up");

    let heap = Heap::start();
    let (_, by_bytes) = allocated(&heap, bitmap(&source).to_bytes()).await;
    let (uri, by_uri) = allocated(&heap, bitmap(&source).to_data_uri()).await;
    drop(heap);

    let uri = uri.expect("the data URI");
    assert!(uri == expected, "the data URI is not the bytes' base64");
    let extra = by_uri.saturating_sub(by_bytes);
    let len = uri.len() as u64;
    assert!(
        extra < len + len / 2,
        "a {len}-byte data URI allocated {by_uri} bytes, {by_bytes} for the image: {extra} \
         more, where one buffer is {len}"
    );
}

/// MEM-003: base64 is encoded into one buffer of its final size.
#[tokio::test]
async fn mem_audit_an_image_base64_is_written_in_one_buffer() {
    let _lock = exclusive().await;
    let source = source();
    let output = bitmap(&source).to_bytes().await.expect("a warm-up");
    let expected = base64::engine::general_purpose::STANDARD.encode(&output);
    bitmap(&source).to_base64().await.expect("a warm-up");

    let heap = Heap::start();
    let (_, by_bytes) = allocated(&heap, bitmap(&source).to_bytes()).await;
    let (encoded, by_base64) = allocated(&heap, bitmap(&source).to_base64()).await;
    drop(heap);

    let encoded = encoded.expect("the base64");
    assert!(encoded == expected, "the base64 is not the bytes' base64");
    let extra = by_base64.saturating_sub(by_bytes);
    let len = encoded.len() as u64;
    assert!(
        extra < len + len / 2,
        "{len} bytes of base64 allocated {by_base64} bytes, {by_bytes} for the image: {extra} \
         more, where one buffer is {len}"
    );
}

/// MEM-003: a response carries the encoded bytes it was handed.
#[tokio::test]
async fn mem_audit_an_image_response_carries_its_bytes_without_copying() {
    let _lock = exclusive().await;
    let source = source();
    let output = bitmap(&source).to_bytes().await.expect("a warm-up");
    bitmap(&source).to_response().await.expect("a warm-up");

    let heap = Heap::start();
    let (_, by_bytes) = allocated(&heap, bitmap(&source).to_bytes()).await;
    let (response, by_response) = allocated(&heap, bitmap(&source).to_response()).await;
    drop(heap);

    let response = response.expect("the response");
    assert!(
        response.body() == output.as_slice(),
        "the response body is not the bytes"
    );
    let extra = by_response.saturating_sub(by_bytes);
    let len = output.len() as u64;
    assert!(
        extra < len / 2,
        "a {len}-byte response allocated {by_response} bytes, {by_bytes} for the image: {extra} \
         more"
    );
}

/// MEM-003: saving writes the encoded bytes it owns.
#[tokio::test]
async fn mem_audit_an_image_save_writes_its_bytes_without_copying() {
    let _lock = exclusive().await;
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("out.bmp");
    let source = source();
    let output = bitmap(&source).to_bytes().await.expect("a warm-up");
    bitmap(&source).save(&path).await.expect("a warm-up");

    let heap = Heap::start();
    let (_, by_bytes) = allocated(&heap, bitmap(&source).to_bytes()).await;
    let (saved, by_save) = allocated(&heap, bitmap(&source).save(&path)).await;
    drop(heap);

    saved.expect("the save");
    assert!(
        std::fs::read(&path).expect("the saved file") == output,
        "the saved file is not the bytes"
    );
    let extra = by_save.saturating_sub(by_bytes);
    let len = output.len() as u64;
    assert!(
        extra < len / 2,
        "saving {len} bytes allocated {by_save} bytes, {by_bytes} for the image: {extra} more"
    );
}

/// MEM-003: storing on a disk costs what putting the encoded bytes on that
/// disk costs, with no copy of them on the way.
#[cfg(all(feature = "filesystem", feature = "testing"))]
#[tokio::test]
async fn mem_audit_an_image_store_puts_its_bytes_without_copying() {
    use suprnova::{DiskExt, Storage};

    let _lock = exclusive().await;
    let _storage = Storage::fake();
    Storage::register_memory("mem-audit-images");
    let disk = Storage::disk("mem-audit-images").expect("a disk");
    let source = source();
    let output = bitmap(&source).to_bytes().await.expect("a warm-up");
    disk.put("put.bmp", output.clone())
        .await
        .expect("a warm-up");
    bitmap(&source)
        .store_as("", "stored.bmp", Some("mem-audit-images"))
        .await
        .expect("a warm-up");
    bitmap(&source)
        .store("", Some("mem-audit-images"))
        .await
        .expect("a warm-up");
    let owned = output.clone();

    let heap = Heap::start();
    let (_, by_bytes) = allocated(&heap, bitmap(&source).to_bytes()).await;
    let (put, by_put) = allocated(&heap, disk.put("put.bmp", owned)).await;
    let (stored_as, by_store_as) = allocated(
        &heap,
        bitmap(&source).store_as("", "stored.bmp", Some("mem-audit-images")),
    )
    .await;
    let (stored, by_store) =
        allocated(&heap, bitmap(&source).store("", Some("mem-audit-images"))).await;
    drop(heap);

    put.expect("the put");
    let stored_as = stored_as.expect("the store_as");
    let stored = stored.expect("the store");
    for path in [&stored_as, &stored] {
        assert!(
            disk.get(path).await.expect("the stored file") == output,
            "{path} is not the bytes"
        );
    }
    let len = output.len() as u64;
    for (name, by_terminal) in [("store_as", by_store_as), ("store", by_store)] {
        let extra = by_terminal.saturating_sub(by_bytes + by_put);
        assert!(
            extra < len / 2,
            "{name} of {len} bytes allocated {by_terminal} bytes, {by_bytes} for the image and \
             {by_put} for the put: {extra} more"
        );
    }
}
