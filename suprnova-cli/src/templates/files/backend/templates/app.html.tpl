<!DOCTYPE html>
<html lang="{{ lang }}">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="suprnova-live-root" content="{{ suprnova::url::root() }}" />
    {# The application's own head content goes here: a favicon, fonts, meta tags. #}
    {{ title }}
    {% if let Ok(url) = "live_async_url"|value::<String> %}
    {% if let Ok(sri) = "live_async_sri"|value::<String> %}
    {% if let Some(nonce) = nonce %}
    <script type="module" src="{{ suprnova::url::root() }}{{ url }}" integrity="{{ sri }}" crossorigin="anonymous" nonce="{{ nonce }}"></script>
    {% else %}
    <script type="module" src="{{ suprnova::url::root() }}{{ url }}" integrity="{{ sri }}" crossorigin="anonymous"></script>
    {% endif %}
    {% endif %}
    {% endif %}
    {{ head }}
    {# An application script runs under the page's policy with the request's nonce:
       {% if let Some(nonce) = nonce %}<script nonce="{{ nonce }}" src="/js/app.js"></script>{% endif %} #}
  </head>
  <body>
    {{ body }}
  </body>
</html>
