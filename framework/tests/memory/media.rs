//! MEM-003 and MEM-005 on documentation and processes. Images are in
//! `images` and files in `files`, behind the features they need.

use suprnova::Process;
use suprnova::content::{DocsBuildConfig, build_docs};

use crate::support::{Heap, exclusive};

/// MEM-005: the docs builder keeps no chapter after writing it, so its
/// peak is one chapter's work, not the whole corpus.
#[tokio::test]
async fn mem_audit_the_docs_builder_keeps_no_chapter() {
    let _lock = exclusive().await;
    let dir = tempfile::tempdir().expect("a directory");
    let source = dir.path().join("manual");
    let output = dir.path().join("out");
    std::fs::create_dir_all(&source).expect("mkdir");
    let mut toc = String::from("# Documentation\n\n");
    let link = format!("[x](https://example.com/{})\n", "a".repeat(200));
    for chapter in 0..40 {
        std::fs::write(
            source.join(format!("chapter-{chapter}.md")),
            format!("# Chapter {chapter}\n\n{}", link.repeat(4_000)),
        )
        .expect("write a chapter");
        toc.push_str(&format!("- [Chapter {chapter}](chapter-{chapter}.md)\n"));
    }
    let toc_file = source.join("documentation.md");
    std::fs::write(&toc_file, toc).expect("write the toc");

    let heap = Heap::start();
    let start = heap.live();
    build_docs(DocsBuildConfig {
        source_dir: source,
        output_dir: output.clone(),
        toc_file,
    })
    .await
    .expect("the docs build");
    let peak = heap.peak() - start;
    drop(heap);

    let mut html = 0;
    for chapter in 0..40 {
        let json = std::fs::read_to_string(output.join(format!("chapter-{chapter}.json")))
            .expect("a chapter");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        html += value["html"].as_str().map_or(0, str::len);
    }
    assert!(
        peak < html / 2,
        "the build peaked at {peak} bytes over {html} bytes of HTML"
    );
}

/// MEM-003: a process's output is held once in its result.
#[cfg(unix)]
#[tokio::test]
async fn mem_audit_process_output_is_held_once() {
    let _lock = exclusive().await;
    const SIZE: usize = 8 * 1024 * 1024;
    Process::shell("printf warm")
        .run()
        .await
        .expect("a warm-up");
    let heap = Heap::start();
    let start = heap.live();
    let result = Process::shell(format!("head -c {SIZE} /dev/zero"))
        .run()
        .await
        .expect("the process");
    let peak = heap.peak() - start;
    drop(heap);
    assert_eq!(result.output_bytes().len(), SIZE);
    assert!(
        peak < 2 * SIZE,
        "capturing {SIZE} bytes peaked at {peak} bytes"
    );
}

/// MEM-005: on a prose manual, the docs builder's peak is one chapter's
/// work plus the catalog it writes and returns, whose search index holds
/// every chapter's text by design: a 40-chapter build peaks within the
/// catalog's size, twice over, of a one-chapter build. Keeping every
/// chapter's HTML, as the builder did, adds 39 chapters to that.
#[tokio::test]
async fn mem_audit_the_docs_builder_peaks_at_one_chapter_and_the_catalog() {
    let _lock = exclusive().await;
    let paragraph = "The framework renders the page, and the **browser** keeps the \
                     `state` it was given, as the [guide](guide.md) describes. "
        .repeat(12);
    let mut chapter = String::from("# Chapter\n\n");
    for section in 0..40 {
        chapter.push_str(&format!("## Section {section}\n\n{paragraph}\n\n"));
    }

    let mut peaks = Vec::new();
    let mut catalog_json = 0;
    for chapters in [1, 40] {
        let dir = tempfile::tempdir().expect("a directory");
        let source = dir.path().join("manual");
        let output = dir.path().join("out");
        std::fs::create_dir_all(&source).expect("mkdir");
        let mut toc = String::from("# Documentation\n\n");
        for n in 0..chapters {
            std::fs::write(source.join(format!("chapter-{n}.md")), &chapter)
                .expect("write a chapter");
            toc.push_str(&format!("- [Chapter {n}](chapter-{n}.md)\n"));
        }
        let toc_file = source.join("documentation.md");
        std::fs::write(&toc_file, toc).expect("write the toc");

        let heap = Heap::start();
        let start = heap.live();
        let catalog = build_docs(DocsBuildConfig {
            source_dir: source,
            output_dir: output.clone(),
            toc_file,
        })
        .await
        .expect("the docs build");
        peaks.push(heap.peak() - start);
        drop(catalog);
        drop(heap);
        catalog_json = std::fs::metadata(output.join("catalog.json"))
            .expect("the catalog")
            .len() as usize;
    }
    assert!(
        peaks[1] < peaks[0] + 2 * catalog_json,
        "40 chapters peaked at {} bytes, one at {}, and the catalog is {catalog_json}",
        peaks[1],
        peaks[0]
    );
}
