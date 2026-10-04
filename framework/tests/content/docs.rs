use suprnova::content::{DocsBuildConfig, build_docs};

#[tokio::test]
async fn docs_builder_emits_catalog_and_rewrites_markdown_links() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let out = tmp.path().join("out");

    tokio::fs::create_dir_all(&src).await.unwrap();
    tokio::fs::write(
        src.join("documentation.md"),
        "- [Getting Started](getting-started.md)\n- [Configuration](configuration.md)\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        src.join("getting-started.md"),
        "# Getting Started\n\nRead [configuration](configuration.md).\n",
    )
    .await
    .unwrap();
    tokio::fs::write(src.join("configuration.md"), "# Configuration\n")
        .await
        .unwrap();

    build_docs(DocsBuildConfig {
        source_dir: src.clone(),
        output_dir: out.clone(),
        toc_file: src.join("documentation.md"),
    })
    .await
    .unwrap();

    let chapter = tokio::fs::read_to_string(out.join("getting-started.json"))
        .await
        .unwrap();
    assert!(chapter.contains("/docs/configuration"));

    let catalog = tokio::fs::read_to_string(out.join("catalog.json"))
        .await
        .unwrap();
    assert!(catalog.contains("Getting Started"));
    assert!(catalog.contains("\"previous\":null"));
    assert!(catalog.contains("\"next\":\"configuration\""));
}

async fn write(path: &std::path::Path, text: &str) {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.unwrap();
    }
    tokio::fs::write(path, text).await.unwrap();
}

/// DRIVERS-009: two chapters in different directories with the same file
/// name map to one slug, so one would overwrite the other's `<slug>.json`
/// while the catalog still listed both. The build refuses, naming both
/// files, and writes nothing.
#[tokio::test]
async fn docs_builder_refuses_two_chapters_that_share_a_slug() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let out = tmp.path().join("out");
    write(
        &src.join("documentation.md"),
        "- [Guide setup](guide/setup.md)\n- [API setup](api/setup.md)\n",
    )
    .await;
    write(&src.join("guide/setup.md"), "# Guide setup\n").await;
    write(&src.join("api/setup.md"), "# API setup\n").await;

    let error = build_docs(DocsBuildConfig {
        source_dir: src.clone(),
        output_dir: out.clone(),
        toc_file: src.join("documentation.md"),
    })
    .await
    .expect_err("two chapters with one slug must not build");
    let message = error.to_string();
    assert!(
        message.contains("guide/setup.md") && message.contains("api/setup.md"),
        "the error names both chapters: {message}"
    );
    assert!(
        !out.join("setup.json").exists(),
        "nothing is written when the table of contents is refused"
    );
}

/// DRIVERS-009: a chapter named `catalog.md` would have its artifact
/// overwritten by the build's own `catalog.json`.
#[tokio::test]
async fn docs_builder_refuses_a_chapter_whose_slug_is_the_catalog() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    write(&src.join("documentation.md"), "- [Catalog](catalog.md)\n").await;
    write(&src.join("catalog.md"), "# Catalog\n").await;

    let error = build_docs(DocsBuildConfig {
        source_dir: src.clone(),
        output_dir: tmp.path().join("out"),
        toc_file: src.join("documentation.md"),
    })
    .await
    .expect_err("a chapter cannot take the catalog's file name");
    assert!(error.to_string().contains("catalog.md"), "{error}");
}

/// A table of contents may list one chapter in two sections (the manual
/// lists `frontend.md` under Getting Started and again as the Frontend
/// overview). That is one chapter, not a collision: it builds, and the
/// catalog keeps one entry per listing so consumers can walk it by
/// position.
#[tokio::test]
async fn docs_builder_accepts_one_chapter_listed_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let out = tmp.path().join("out");
    write(
        &src.join("documentation.md"),
        "- [Frontend](frontend.md)\n- [Pages](pages.md)\n- [Overview](./frontend.md)\n",
    )
    .await;
    write(&src.join("frontend.md"), "# Frontend\n").await;
    write(&src.join("pages.md"), "# Pages\n").await;

    let catalog = build_docs(DocsBuildConfig {
        source_dir: src.clone(),
        output_dir: out.clone(),
        toc_file: src.join("documentation.md"),
    })
    .await
    .expect("one chapter listed twice still builds");
    let slugs: Vec<&str> = catalog.chapters.iter().map(|c| c.slug.as_str()).collect();
    assert_eq!(slugs, ["frontend", "pages", "frontend"]);
    assert!(out.join("frontend.json").exists());
}
