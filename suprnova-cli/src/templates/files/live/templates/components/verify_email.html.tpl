{% import "suprnova-ui/button/button.html" as button %}
<div class="kit-stack">
<p>We sent a verification link to <strong>{{ email }}</strong>. Open it to finish setting up your account.</p>
<p class="kit-muted">No email? Check your spam folder, or ask for a new link.</p>
<p class="kit-form-status" live:error.live.polite="resend"></p>
{% call button::button("Send a new link", action="resend") %}{% endcall %}
</div>
