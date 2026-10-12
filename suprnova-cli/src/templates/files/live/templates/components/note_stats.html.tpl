{% import "suprnova-ui/card/card.html" as card %}
{% import "suprnova-ui/stat-card/stat-card.html" as stat %}
{% import "suprnova-ui/list-group/list-group.html" as list %}
{% import "suprnova-ui/empty-state/empty-state.html" as empty %}
{% import "suprnova-ui/button/button.html" as button %}
{% call card::card("note-stats", "Your notes", kind="section") %}
<div class="kit-stats">
{% call stat::stat_card("stat-notes", "Notes", notes) %}{% endcall %}
{% call stat::stat_card("stat-today", "Written today", written_today) %}{% endcall %}
</div>
<h3 class="kit-subtitle">Recent notes</h3>
{% if recent.is_empty() %}
{% call empty::empty_state("recent-empty", "empty", "No notes yet", text="Your five newest notes show here.") %}{% call button::link_button("Write one", crate::live::href("/notes")) %}{% endcall %}{% endcall %}
{% else %}
{% call list::list_group("recent-notes", "Recent notes") %}{% for note in recent %}{% call list::list_item(note.key) %}<a href="{{ suprnova::url::root() }}/notes/{{ note.id }}">{{ note.title }}</a> <time class="kit-muted" datetime="{{ note.created_at }}">{{ note.created_on }}</time>{% endcall %}{% endfor %}{% endcall %}
{% endif %}
{% call card::card_actions() %}{% call button::button("Refresh", action="refresh", variant="secondary") %}{% endcall %}{% endcall %}
{% endcall %}
