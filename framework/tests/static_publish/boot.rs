//! The real server must accept health requests while its boot publication waits.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use suprnova::static_publish::{PageObject, PublishConfig, Publisher, StaticPublication};
use suprnova::{FrameworkError, HttpResponse, InertiaProps, Request, Router, Server, Storage};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc};

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct BootProps {
    #[inertia(public)]
    title: String,
}

#[tokio::test]
async fn pub_003_listener_health_survives_stalled_and_failed_boot_publication()
-> Result<(), Box<dyn std::error::Error>> {
    if crate::own_process_async::delegate(
        module_path!(),
        "pub_003_listener_health_survives_stalled_and_failed_boot_publication",
    )
    .await
    {
        return Ok(());
    }
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&[
        "APP_ENV",
        "CACHE_DRIVER",
        "QUEUE_DRIVER",
        "MAIL_DRIVER",
        "RATE_LIMIT_DRIVER",
    ]);
    crate::env_snapshot::set_env("APP_ENV", Some("testing"));
    crate::env_snapshot::set_env("CACHE_DRIVER", Some("memory"));
    crate::env_snapshot::set_env("QUEUE_DRIVER", Some("memory"));
    crate::env_snapshot::set_env("MAIL_DRIVER", Some("log"));
    crate::env_snapshot::set_env("RATE_LIMIT_DRIVER", Some("memory"));
    let _container = suprnova::testing::TestContainer::fake();
    let probe = TcpListener::bind("127.0.0.1:0").await?;
    let addr = probe.local_addr()?;
    drop(probe);

    let (entered_tx, mut entered_rx) = mpsc::channel(1);
    let release = Arc::new(Notify::new());
    let handler_release = release.clone();
    let router: Router = Router::new()
        .get("/public", move |_request: Request| {
            let release = handler_release.clone();
            let entered = entered_tx.clone();
            async move {
                let listener_already_open = TcpStream::connect(addr).await.is_ok();
                let _ = entered.send(listener_already_open).await;
                release.notified().await;
                Err::<HttpResponse, _>(
                    FrameworkError::internal("deliberate failed boot publication").into(),
                )
            }
        })
        .into();
    let router = router.public_page::<BootProps>("/public", "Public")?;
    Storage::register_memory("boot-publication");
    let config = PublishConfig {
        timeout: Duration::from_secs(20),
        revalidate: Duration::from_secs(3600),
        ..PublishConfig::default()
    };
    let publisher = Publisher::new("boot-publication", "boot-v1", config)?;
    publisher.publish(vec![PageObject::new("/public", Bytes::from_static(b"<html>previous</html>"), Bytes::from_static(br#"{"component":"Public","props":{"title":"previous"},"url":"/public","version":"boot-v1"}"#))?]).await?;
    let disk = Storage::disk("boot-publication")?;
    let before = disk.read("_pub/manifest.json").await?.to_bytes();
    let runtime = StaticPublication::install(&router, publisher)?;
    let server = tokio::spawn(
        Server::new(router)
            .host("127.0.0.1")
            .port(addr.port())
            .run(),
    );

    let listener_was_open = tokio::time::timeout(Duration::from_secs(10), entered_rx.recv())
        .await?
        .ok_or("publication did not start")?;
    assert!(
        listener_was_open,
        "publication began before its listener opened"
    );
    assert!(
        !server.is_finished(),
        "stalled publication stopped the server"
    );
    let (status, _, _) =
        crate::http_wire::request(addr, "GET", "/_suprnova/health/live", &[]).await;
    assert_eq!(status, 200, "health was blocked by the stalled publication");
    assert_eq!(disk.read("_pub/manifest.json").await?.to_bytes(), before);

    release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = runtime.status().await;
            if status.last_outcome.contains("HTTP 500") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(
        disk.read("_pub/manifest.json").await?.to_bytes(),
        before,
        "failed boot publication switched the manifest"
    );
    assert!(
        !server.is_finished(),
        "failed publication stopped the server"
    );
    let (status, _, _) =
        crate::http_wire::request(addr, "GET", "/_suprnova/health/live", &[]).await;
    assert_eq!(status, 200, "failed publication stopped health responses");
    server.abort();
    let _ = server.await;
    Ok(())
}
