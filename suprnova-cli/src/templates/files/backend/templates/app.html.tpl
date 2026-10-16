<!DOCTYPE html>
<html lang="{{ lang }}">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    {# The application's own head content goes here: a favicon, fonts, meta tags. #}
    {{ title }}
    {{ head }}
    {# An application script runs under the page's policy with the request's nonce:
       {% if let Some(nonce) = nonce %}<script nonce="{{ nonce }}" src="/js/app.js"></script>{% endif %} #}
  </head>
  <body>
    {{ body }}
  </body>
</html>
