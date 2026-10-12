<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{% if frame.title.is_empty() %}{{ frame.app_name }}{% else %}{{ frame.title }} - {{ frame.app_name }}{% endif %}</title>
{% if let Some(bootstrap) = frame.bootstrap %}{{ bootstrap|trusted_html }}{% endif %}
{# The library components the pages use, served from templates/suprnova-ui/ by try_live_ui_assets(). #}
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/header-bar/header-bar.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/account-menu/account-menu.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/toast/toast.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/card/card.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/button/button.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/field/field.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/input/input.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/password-input/password-input.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/checkbox/checkbox.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/textarea/textarea.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/search-input/search-input.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/form-actions/form-actions.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/stat-card/stat-card.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/list-group/list-group.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/load-more/load-more.css">
<link rel="stylesheet" href="{{ frame.root }}/suprnova-ui/empty-state/empty-state.css">
{# The kit's own page layout, served from public/css/app.css. #}
<link rel="stylesheet" href="{{ frame.root }}/css/app.css">
<script type="module" src="{{ frame.root }}/suprnova-ui/toast/toast.js"></script>
<script type="module" src="{{ frame.root }}/suprnova-ui/password-input/password-input.js"></script>
