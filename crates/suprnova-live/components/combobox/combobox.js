// sn-combobox - the accessible combobox pattern over a native input and a
// server-rendered listbox (FORM-008). Light DOM: the input, the datalist
// and the listbox are the server's markup. The input carries the value, so
// nothing here is form-associated and the control works with this file
// blocked (the datalist gives the same options natively).
//
// A remote listbox (data-sn-remote) holds the options the server rendered
// for the query in its data-sn-query. The element shows all of them, as the
// server chose them, while that query is the input's current text
// (FORM-012), and keeps a listbox rendered for any other text hidden until
// a newer render arrives (stale suppression, FORM-008);
// SuprnovaCombobox.acceptsResults decides that alone so a fixture can prove
// it without a document. A local listbox holds a fixed list, which the
// element filters by the typed text.
//
// Every write below happens only when the value changes. The listbox
// observer watches the hidden attribute that a morph resets, and setting an
// attribute queues a record even when the value is unchanged, so an
// unconditional write would feed the observer forever (FORM-011).
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
  const setAttribute = (element, name, value) => {
    if (element.getAttribute(name) !== value) element.setAttribute(name, value);
  };
  const removeAttribute = (element, name) => {
    if (element.hasAttribute(name)) element.removeAttribute(name);
  };
  const setHidden = (element, hidden) => {
    if (element.hidden !== hidden) element.hidden = hidden;
  };
  customElements.define(
    "sn-combobox",
    class extends HTMLElement {
      // One connection's listeners and observers (UI-020): a disconnect
      // aborts them, and a morph that moves the element keeps them.
      #connection = null;

      connectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
        const input = this.querySelector("input[role=combobox]");
        const listbox = this.querySelector("[role=listbox]");
        if (!input || !listbox) return;
        const connection = new AbortController();
        this.#connection = connection;
        const { signal } = connection;
        const observe = (target, callback, options) => {
          const observer = new MutationObserver(callback);
          observer.observe(target, options);
          signal.addEventListener("abort", () => observer.disconnect(), { once: true });
        };
        // The datalist stays in the markup for the script-free case; the
        // upgraded input stops pointing at it, and a morph that restores the
        // attribute is answered the same way.
        removeAttribute(input, "list");
        observe(input, () => removeAttribute(input, "list"), { attributes: true, attributeFilter: ["list"] });
        const sequence = new QuerySequence();
        let active = -1;
        // After a selection the popup stays closed until the user types
        // again, even though the selection's own model round-trip re-renders
        // the listbox.
        let selected = false;
        const options = () => Array.from(listbox.querySelectorAll("[role=option]"));
        const visible = () => options().filter((option) => !option.hidden);
        const setExpanded = (expanded) => {
          setAttribute(input, "aria-expanded", String(expanded));
          setHidden(listbox, !expanded);
          if (!expanded) {
            active = -1;
            removeAttribute(input, "aria-activedescendant");
            for (const option of options()) removeAttribute(option, "data-sn-active");
          }
        };
        const render = () => {
          const query = input.value.trim();
          const remote = listbox.hasAttribute("data-sn-remote");
          if (remote && !acceptsResults(query, (listbox.getAttribute("data-sn-query") ?? "").trim())) {
            setExpanded(false);
            return;
          }
          const needle = query.toLowerCase();
          let shown = 0;
          for (const option of options()) {
            const match = remote || needle === "" || option.textContent.toLowerCase().includes(needle);
            setHidden(option, !match);
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
              setAttribute(option, "data-sn-active", "");
              setAttribute(input, "aria-activedescendant", option.id);
              option.scrollIntoView({ block: "nearest" });
            } else {
              removeAttribute(option, "data-sn-active");
            }
          });
        };
        const select = (option) => {
          for (const other of options()) setAttribute(other, "aria-selected", String(other === option));
          input.value = option.textContent.trim();
          input.dispatchEvent(new Event("input", { bubbles: true }));
          input.dispatchEvent(new Event("change", { bubbles: true }));
          selected = true;
          setExpanded(false);
        };
        input.addEventListener(
          "input",
          () => {
            selected = false;
            sequence.ask();
            render();
          },
          { signal },
        );
        input.addEventListener("focus", render, { signal });
        input.addEventListener(
          "blur",
          () =>
            setTimeout(() => {
              if (!signal.aborted) setExpanded(false);
            }, 100),
          { signal },
        );
        input.addEventListener(
          "keydown",
          (event) => {
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
          },
          { signal },
        );
        listbox.addEventListener(
          "mousedown",
          (event) => {
            const option = event.target instanceof Element ? event.target.closest("[role=option]") : null;
            if (option) {
              event.preventDefault();
              select(option);
            }
          },
          { signal },
        );
        // A morph replaces the options, rewrites the query, and resets the
        // listbox's hidden attribute; re-render and re-apply the active option.
        observe(
          listbox,
          () => {
            if (selected) {
              setExpanded(false);
              return;
            }
            if (document.activeElement !== input) return;
            render();
            if (active >= 0 && !listbox.hidden) activate(active);
          },
          { attributes: true, attributeFilter: ["data-sn-query", "hidden"], childList: true },
        );
        setAttribute(this, "data-sn-upgraded", "");
      }

      // A morph that moves the element keeps its connection and state.
      connectedMoveCallback() {}

      disconnectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
      }
    },
  );
})();
