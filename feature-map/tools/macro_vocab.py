"""Arguments each proc-macro's parser accepts, curated by reading the parsers.

Each entry is (macro, argument syntax, parser file, parser fn, keyword).
The keyword locates the evidence line in kw.json (from macro_keywords.py);
a lookup that fails is a hard error, so an entry cannot outlive its code.

Keywords the parsers match that are NOT listed here were reviewed and are
internal (listed in INTERNAL with the reason).
"""
import json
import sys
from pathlib import Path

M = "suprnova-macros/src/"
V = [
    # #[suprnova::model(...)]
    *[("model", f"`{k}`", "model/parse.rs", "parse", k) for k in (
        "table", "primary_key", "key_type", "auto_increment", "connection", "fillable",
        "guarded", "casts", "timestamps", "created_at", "updated_at", "soft_deletes",
        "soft_deletes_column", "appends", "hidden", "visible", "mutators", "touches",
        "relations", "morph_type", "observers")],
    ("model", "`unique_id = \"uuid\" | \"uuid_v7\" | \"uuid_v4\" | \"ulid\"`", "model/parse.rs", "parse", "unique_id"),
    *[("model", f"relation option `{k}`", "model/parse.rs", "parse_relation_options", k) for k in (
        "fk", "lk", "with_pivot", "with_timestamps", "with_default", "scope", "name",
        "morph_name", "targets", "first_key", "second_key", "second_local_key", "pivot_table",
        "pivot_foreign_key", "pivot_related_key", "related_key", "target_morph_type")],
    # #[suprnova::observer(M)] impl: lifecycle method names it wires
    *[("observer", f"method `{k}`", "observer.rs", "emit_adapter_call", k) for k in (
        "retrieving", "retrieved", "created", "saved", "updated", "deleted", "trashed",
        "restored", "replicating", "force_deleting", "force_deleted")],
    *[("observer", f"cancellable method `{k}`", "observer.rs", "emit_cancellable_adapter_call", k) for k in (
        "creating", "saving", "updating", "deleting", "restoring")],
    # #[derive(Data)] + #[data(...)] / #[json_resource]
    *[("Data", f"struct `#[data({k})]`", "data.rs", "parse_struct_options", k) for k in (
        "allow_unknown_fields", "deny_unknown_fields", "authorize", "custom_authorize",
        "auto_lazy", "id_field", "max_body_bytes")],
    ("Data", "struct `#[json_resource(\"type\")]`", "data.rs", "parse_struct_options", "json_resource"),
    *[("Data", f"field `#[data({k})]`", "data.rs", "parse_field_options", k) for k in (
        "input_only", "output_only", "allow_include", "from_route_param")],
    ("Data", "field `#[data(lazy = \"inertia\" | \"deferred\" | \"closure\" | \"when_loaded\")]`",
     "data.rs", "parse_field_options", "lazy"),
    # #[derive(MultipartRequest)]
    *[("MultipartRequest", f"`#[multipart({k})]`", "multipart.rs", "expand_inner", k) for k in (
        "max_body_bytes", "custom_hooks")],
    ("MultipartRequest", "field `#[field(max_count = N)]`", "multipart.rs", "expand_inner", "max_count"),
    # #[derive(FormRequest)] / #[request]
    *[("FormRequest", f"`#[form_request({k})]`", "request.rs", "parse_form_request_attrs", k) for k in (
        "max_body_bytes", "custom_hooks")],
    # #[derive(NotificationMailable)] #[mail(...)]
    *[("NotificationMailable", f"`#[mail({k})]`", "notification_mail.rs", "parse_mail_attr", k) for k in (
        "subject", "from", "from_name", "reply_to", "cc", "bcc", "html", "text",
        "html_template", "text_template")],
    # #[derive(Command)] #[console(...)] and #[command(...)]
    *[("Command", f"`#[console({k})]`", "console_derive.rs", "parse_attrs", k) for k in ("name", "description")],
    *[("command", f"`{k} = \"...\"`", "command.rs", "parse_attrs", k) for k in ("name", "description")],
    # #[derive(Factory)] #[factory(name = ...)]
    ("Factory", "`#[factory(name = ...)]`", "factory.rs", "parse_attrs", "name"),
    # #[domain_error(status = .., message = ..)]
    *[("domain_error", f"`{k}`", "domain_error.rs", "parse_attrs", k) for k in ("status", "message")],
    # #[injectable] + #[inject] on fields
    ("injectable", "field `#[inject]`", "injectable.rs", "has_inject_attr", "inject"),
    # #[suprnova::main(...)]
    ("main", "`flavor = \"multi_thread\" | \"current_thread\"`", "main_macro.rs", "parse", "flavor"),
    ("main", "`worker_threads = N`", "main_macro.rs", "parse", "worker_threads"),
    # #[service(impl = T, fake = T)]
    *[("service", f"`{k} = Type`", "service.rs", "parse", k) for k in ("impl", "fake")],
    # #[suprnova_test(migrator = M)]
    ("suprnova_test", "`migrator = Type`", "suprnova_test.rs", "parse", "migrator"),
    # #[suprnova::view(path = "...")]
    ("view", "`path = \"...\"`", "view.rs", "parse_view_path", "path"),
    # #[derive(LiveComponent)] #[live(...)] on the struct
    *[("LiveComponent", f"struct `#[live({k})]`", "live/attrs.rs", "parse_component_args", k) for k in (
        "name", "view", "component_version", "state_schema_version", "action_schema_version",
        "checker_contract_version", "minimum_protocol_version", "events", "effects",
        "refresh_on_promote", "streams")],
    ("LiveComponent", "struct `#[live(streams(stream(...), ...))]`", "live/attrs.rs", "parse_stream_list", "stream"),
    ("LiveComponent", "field `#[validate(...)]` (list form, passed to `validator::Validate`)", "live/attrs.rs", "parse_field_args", "validate"),
    *[("LiveComponent", f"stream `stream({k} = ...)`", "live/attrs.rs", "parse_stream_args", k) for k in (
        "name", "topics", "events", "targets", "fanout", "modes", "reconnect", "resume_attempts")],
    ("LiveComponent", "stream `targets` values `self`, `parent`, `child`, `document`", "live/attrs.rs", "parse_stream_target", "document"),
    ("LiveComponent", "stream `modes` values `sse`, `websocket`", "live/attrs.rs", "parse_stream_mode", "websocket"),
    *[("LiveComponent", f"field `#[{k}]`", "live/attrs.rs", "parse_field_args", k) for k in (
        "public", "locked", "server_only", "session", "secret", "model", "transient", "url", "upload")],
    *[("LiveComponent", f"field `#[model({k})]`", "live/attrs.rs", "parse_model_args", k) for k in (
        "immediate", "change", "blur", "submit", "debounce", "transient")],
    *[("LiveComponent", f"field `#[url({k})]`", "live/attrs.rs", "parse_url_args", k) for k in (
        "key", "omit_default")],
    ("LiveComponent", "field `#[url(mode = \"reflect\" | \"navigate\")]`", "live/attrs.rs", "parse_url_args", "mode"),
    ("LiveComponent", "field `#[upload(policy = ...)]`", "live/attrs.rs", "parse_upload_args", "policy"),
    *[("LiveComponent", f"method `#[action({k})]`", "live/attrs.rs", "parse_action_args", k) for k in (
        "name", "version")],
    ("LiveComponent", "method `#[action(authorize = \"public\" | \"current\")]`", "live/attrs.rs", "parse_action_args", "authorize"),
    ("LiveComponent", "method `#[action(validate = \"none\" | \"whole\" | \"arguments\" | \"all\")]`", "live/attrs.rs", "parse_action_args", "validate"),
    ("LiveComponent", "method `#[action(transaction = \"none\" | \"required\")]`", "live/attrs.rs", "parse_action_args", "transaction"),
    ("LiveComponent", "method `#[validate(action = ...)]`", "live/attrs.rs", "parse_validation_hook_args", "action"),
    *[("live", f"method `#[{k}]` in a `#[live]` impl", "live/live_impl.rs", "expand", k) for k in (
        "mount", "action", "computed", "validate", "params_changed", "lazy_complete")],
]

INTERNAL = {
    "__eager / __pivot": "hidden eager-load and pivot carrier fields the model macro injects",
    "__suprnova_live": "internal twin of #[live] used by generated code",
    "primitive type names (bool, i64, u64, f32, ...)": "type classification inside Data and Live codecs",
    "self / super": "path rewriting in #[suprnova::view] field access",
    "suprnova": "crate-path detection in #[handler] parameter classification",
    "id": "default key name inside relation code generation",
    "static": "`'static` detection in #[service]",
    "suprnova_test": "the attribute recognising itself",
    "form_request / multipart / data / factory / mail / console / live / field": "the attribute names themselves",
    "bool (policy.rs)": "return-type classification in #[policy]",
}


def main():
    root = Path(sys.argv[1])
    kw = json.loads(Path(sys.argv[2]).read_text())
    idx = {}
    for r in kw:
        idx.setdefault((r["file"], r["fn"], r["keyword"]), r["line"])
    out = {}
    for macro, syntax, f, fn, k in V:
        key = (M + f, fn, k)
        if key not in idx:
            raise SystemExit(f"no evidence for {macro} {syntax}: {key}")
        out.setdefault(macro, []).append({"syntax": syntax, "at": f"{M}{f}:{idx[key]}"})
    json.dump({"arguments": out, "internal": INTERNAL}, sys.stdout, indent=1)


if __name__ == "__main__":
    main()
