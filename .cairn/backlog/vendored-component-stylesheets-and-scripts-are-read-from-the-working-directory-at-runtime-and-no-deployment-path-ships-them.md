# Vendored component stylesheets and scripts are read from the working directory at runtime, and no deployment path ships them

Surfaced from: UI-017
Outside because: live-protocol-bounds bounds what one Live request and response carry (LIVE-028 to LIVE-030); where the framework reads vendored component assets from and how a deployment ships them is not part of that work.
Captured: 2026-09-17T11:53:47.719Z

Found 2026-09-17 in the adversarial review of Live and the component library before 2.1.0. Router::try_live_ui_assets reads base_path("templates")/suprnova-ui on every request (framework/src/live/routes.rs, framework/src/live/ui_assets.rs), and base_path is APP_BASE_PATH or the working directory, while views are compiled into the binary and runtime artifacts are embedded. The scaffold Dockerfile's runtime image copies only the binary and public/, so every /suprnova-ui/<component>/<file> answers 404 there with no build or boot error, and manual/live.md never says the directory must ship. suprnova live:assets publishes runtime artifacts only. The dogfood host works around it with set_base_path(env!("CARGO_MANIFEST_DIR")).
