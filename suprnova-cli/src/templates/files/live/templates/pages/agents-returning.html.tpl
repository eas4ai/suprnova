{% extends "live/layouts/app.html" %}
{#
  The answer to an approval decision: a page that sends the browser on to
  the client's redirect URI. The kit's policy carries `form-action 'self'`,
  and a browser applies it to the redirects that follow a form post, so a
  redirect to the client, usually on another origin, would be blocked. A
  meta refresh is a navigation of this page, which `form-action` does not
  govern, and the link serves a browser that does not follow it. The
  address is the authorization server's own, built from a redirect URI the
  client registered, and goes through the attribute escaping.
-#}
{% block head %}
<meta http-equiv="refresh" content="0; url={{ redirect }}">
{% endblock %}
{% block content %}
<h1 class="kit-page-title">Returning you to <bdi>{{ client_name }}</bdi>…</h1>
<p class="kit-lead"><a href="{{ redirect }}">Continue to <bdi>{{ client_name }}</bdi></a></p>
{% endblock %}
