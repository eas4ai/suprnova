{#
  The guest layout: the frame of the sign-in, registration, password and
  verification pages. A page extends it and fills the `content` block.
-#}
<!doctype html>
<html lang="{{ frame.lang }}">
<head>
{% include "live/layouts/head.html" %}
</head>
<body class="kit-guest" data-layout="guest">
<main class="kit-guest-main">
<h1 class="kit-brand"><a href="{{ frame.href("/") }}">{{ frame.app_name }}</a></h1>
{% block content %}{% endblock %}
<nav class="kit-guest-links" aria-label="Account">
{% if let Some(user) = frame.user %}
<span>{{ user.name }}</span>
<form class="kit-inline-form" method="post" action="{{ frame.href("/logout") }}"><input type="hidden" name="_token" value="{{ frame.csrf }}"><button class="kit-link-button" type="submit">Sign out</button></form>
{% else %}
<a href="{{ frame.href("/login") }}">Sign in</a>
<a href="{{ frame.href("/register") }}">Register</a>
{% endif %}
</nav>
</main>
{% include "live/layouts/toast.html" %}
</body>
</html>
