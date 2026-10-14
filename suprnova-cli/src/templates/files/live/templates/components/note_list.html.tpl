{% import "suprnova-ui/card/card.html" as card %}
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/search-input/search-input.html" as search_input %}
{% import "suprnova-ui/button/button.html" as button %}
{% import "suprnova-ui/load-more/load-more.html" as more %}
{% import "suprnova-ui/empty-state/empty-state.html" as empty %}
{% call card::card("note-list", "Your notes", kind="section") %}
<form class="kit-search" role="search" aria-label="Search your notes" live:submit.prevent="search">
{% call field::field("query", "Search") %}{% call search_input::search_input("query", placeholder="Search titles and text", value=query) %}{% endcall %}{% endcall %}
{% call button::button("Search", kind="submit", action="search", variant="secondary") %}{% endcall %}
</form>
{% if notes.is_empty() %}
{% if search.is_empty() %}
{% call empty::empty_state("notes-empty", "empty", "No notes yet", text="Write your first note with the form on this page.") %}{% endcall %}
{% else %}
{% call empty::empty_state("notes-empty", "no-results", "No notes match", text=component.no_results()) %}{% call button::button("Clear the search", action="clear_search", variant="secondary") %}{% endcall %}{% endcall %}
{% endif %}
{% else %}
{% call more::load_more_list("notes", label="Your notes") %}{% for note in notes %}<li class="kit-note-row" live:key="{{ note.key|live_key }}"><a href="{{ suprnova::url::root() }}/notes/{{ note.id }}">{{ note.title }}</a> <time class="kit-muted" datetime="{{ note.created_at }}">{{ note.created_on }}</time></li>{% endfor %}{% endcall %}
{% call more::load_more("load_more", exhausted=!more) %}{% endcall %}
{% endif %}
{% endcall %}
