{% import "suprnova-ui/toast/toast.html" as toast %}
{# The toast an earlier step kept for this page (src/flash.rs), shown once. #}
{% call toast::toast_region("toasts") %}{% if let Some(kept) = frame.toast %}{% call toast::toast("flash-toast", kept.message, variant=kept.kind) %}{% endcall %}{% endif %}{% endcall %}
