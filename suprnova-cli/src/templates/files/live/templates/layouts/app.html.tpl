{#
  The application layout: the frame of the welcome page, the dashboard, the
  notes pages and the error page. The header bar holds the navigation, the
  account menu holds the signed-in user and the sign-out form, and the
  toast region shows the toast an earlier step kept. A page extends it
  and fills the `content` block.
-#}
{%- import "suprnova-ui/header-bar/header-bar.html" as header -%}
{%- import "suprnova-ui/account-menu/account-menu.html" as menu -%}
{%- import "suprnova-ui/button/button.html" as button -%}
<!doctype html>
<html lang="{{ frame.lang }}">
<head>
{% include "live/layouts/head.html" %}
</head>
<body class="kit-app" data-layout="app">
<div class="kit-topbar">
{% call header::header_bar("site-header", frame.app_name) %}
{% if frame.user.is_some() %}
{% call header::header_link(frame.href("/dashboard"), "Dashboard", current=frame.is_current(Nav::Dashboard)) %}{% endcall %}
{% call header::header_link(frame.href("/notes"), "Notes", current=frame.is_current(Nav::Notes)) %}{% endcall %}
{% endif %}
{% endcall %}
<div class="kit-account">
{% if let Some(user) = frame.user %}
{% call menu::account_menu("account-menu", user.name, user.initials) %}
{% call menu::account_link(frame.href("/dashboard"), "Dashboard") %}{% endcall %}
{% call menu::account_link(frame.href("/notes"), "Notes") %}{% endcall %}
{% call menu::account_sign_out(frame.href("/logout"), frame.csrf) %}{% endcall %}
{% endcall %}
{% else %}
{% call button::link_button("Sign in", frame.href("/login")) %}{% endcall %}
{% call button::link_button("Register", frame.href("/register"), variant="primary") %}{% endcall %}
{% endif %}
</div>
</div>
<main class="kit-app-main">
{% block content %}{% endblock %}
</main>
{% include "live/layouts/toast.html" %}
</body>
</html>
