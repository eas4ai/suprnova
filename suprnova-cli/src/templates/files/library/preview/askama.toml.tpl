# Askama reads this application's own views from `templates/` and the
# library's views from `../components/`, where they sit. Each
# `templates/{namespace}-ui/<component>/<view>.html` is a one-line stub that
# includes the library's view, so nothing is copied: the preview compiles,
# and `suprnova live:check` proves, each view exactly as the library ships it.
[general]
dirs = ["templates", "../components"]
