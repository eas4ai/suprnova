// sn-combobox - the accessible combobox pattern over a native input and a
// server-rendered listbox (FORM-008). Light DOM: the input, the datalist
// and the listbox are the server's markup. The input carries the value, so
// nothing here is form-associated and the control works with this file
// blocked (the datalist gives the same options natively).
//
// Stale suppression: the listbox carries data-sn-query, the query the server
// rendered it for. A listbox whose query is not a prefix-compatible answer to
// the input's current text is older than the text and stays hidden until a
// newer render arrives; SuprnovaCombobox.acceptsResults decides that alone so
// a fixture can prove it without a document.
(function () {
  const acceptsResults = (currentQuery, resultQuery) => {
    if (typeof currentQuery !== "string" || typeof resultQuery !== "string") return false;
    return resultQuery === currentQuery;
  };
  // A monotonic sequence for asynchronous answers: an answer whose sequence
  // is older than the latest question is stale even when its text matches.
  class QuerySequence {
    #latest = 0;
    ask() {
      this.#latest += 1;
      return this.#latest;
    }
    accepts(sequence) {
      return sequence === this.#latest;
    }
  }
  const api = Object.freeze({ QuerySequence, acceptsResults });
  if (typeof globalThis !== "undefined") globalThis.SuprnovaCombobox = api;
  if (typeof customElements === "undefined" || customElements.get("sn-combobox")) return;
  customElements.define(
    "sn-combobox",
    class extends HTMLElement {
      connectedCallback() {
        const input = this.querySelector("input[role=combobox]");
        const listbox = this.querySelector("[role=listbox]");
        if (!input || !listbox) return;
        // The datalist stays in the markup for the script-free case; the
        // upgraded input stops pointing at it, and a morph that restores the
        // attribute is answered the same way.
        input.removeAttribute("list");
        new MutationObserver(() => {
          if (input.hasAttribute("list")) input.removeAttribute("list");
        }).observe(input, { attributes: true, attributeFilter: ["list"] });
        const sequence = new QuerySequence();
        let active = -1;
        // After a selection the popup stays closed until the user types
        // again, even though the selection's own model round-trip re-renders
        // the listbox.
        let selected = false;
        const options = () => Array.from(listbox.querySelectorAll("[role=option]"));
        const visible = () => options().filter((option) => !option.hidden);
        const setExpanded = (expanded) => {
          input.setAttribute("aria-expanded", String(expanded));
          listbox.hidden = !expanded;
          if (!expanded) {
            active = -1;
            input.removeAttribute("aria-activedescendant");
            for (const option of options()) option.removeAttribute("data-sn-active");
          }
        };
        const render = () => {
          if (!acceptsResults(input.value.trim(), (listbox.getAttribute("data-sn-query") ?? "").trim()) && listbox.hasAttribute("data-sn-remote")) {
            setExpanded(false);
            return;
          }
          const needle = input.value.trim().toLowerCase();
          let shown = 0;
          for (const option of options()) {
            const match = needle === "" || option.textContent.toLowerCase().includes(needle);
            option.hidden = !match;
            if (match) shown += 1;
          }
          setExpanded(shown > 0 && document.activeElement === input);
        };
        const activate = (index) => {
          const shown = visible();
          if (shown.length === 0) return;
          active = ((index % shown.length) + shown.length) % shown.length;
          shown.forEach((option, position) => {
            if (position === active) {
              option.setAttribute("data-sn-active", "");
              input.setAttribute("aria-activedescendant", option.id);
              option.scrollIntoView({ block: "nearest" });
            } else {
              option.removeAttribute("data-sn-active");
            }
          });
        };
        const select = (option) => {
          for (const other of options()) other.setAttribute("aria-selected", "false");
          option.setAttribute("aria-selected", "true");
          input.value = option.textContent.trim();
          input.dispatchEvent(new Event("input", { bubbles: true }));
          input.dispatchEvent(new Event("change", { bubbles: true }));
          selected = true;
          setExpanded(false);
        };
        input.addEventListener("input", () => {
          selected = false;
          sequence.ask();
          render();
        });
        input.addEventListener("focus", render);
        input.addEventListener("blur", () => setTimeout(() => setExpanded(false), 100));
        input.addEventListener("keydown", (event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            if (listbox.hidden) render();
            activate(active + 1);
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            activate(active - 1);
          } else if (event.key === "Enter" && active >= 0 && !listbox.hidden) {
            event.preventDefault();
            select(visible()[active]);
          } else if (event.key === "Escape") {
            setExpanded(false);
          }
        });
        listbox.addEventListener("mousedown", (event) => {
          const option = event.target instanceof Element ? event.target.closest("[role=option]") : null;
          if (option) {
            event.preventDefault();
            select(option);
          }
        });
        // A morph replaces the options and resets the input's popup
        // attributes; re-render and re-apply the active option.
        new MutationObserver(() => {
          if (selected) {
            if (!listbox.hidden) setExpanded(false);
            return;
          }
          if (document.activeElement !== input) return;
          render();
          if (active >= 0 && !listbox.hidden) activate(active);
        }).observe(listbox, { attributes: true, attributeFilter: ["data-sn-query", "hidden"], childList: true });
        this.setAttribute("data-sn-upgraded", "");
      }
    },
  );
})();
