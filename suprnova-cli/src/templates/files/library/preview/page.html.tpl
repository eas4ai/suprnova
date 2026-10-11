<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{ title }} - {namespace} preview</title>
{% for href in stylesheets %}<link rel="stylesheet" href="{{ href }}">
{% endfor %}{% for src in scripts %}<script type="module" src="{{ src }}"></script>
{% endfor %}{{ bootstrap|trusted_html }}
</head>
<body>
<main>
<h1>{{ title }}</h1>
{{ island|trusted_html }}
</main>
</body>
</html>
