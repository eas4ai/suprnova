// sn-combobox - the accessible combobox pattern over a native input and a
// server-rendered listbox (FORM-008). Light DOM: the inputs, the datalist
// and the listbox are the server's markup. Two inputs carry the state: the
// hidden value input (data-sn-combobox-value) holds the selected option's
// value, which is what the field means, and the text input holds what the
// user typed, the query a remote listbox answers. A selection writes the
// option's data-sn-value to the first and its label to the second; editing
// the text afterwards clears the value, so a stale selection never stays
// behind text that no longer names it. Nothing here is form-associated, and
// with this file blocked the datalist still offers the labels as text.
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
        const valueInput = this.querySelector("input[data-sn-combobox-value]");
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
        // The value input is bound like any field: the model directive hears
        // its input and change events, so a write announces itself the same
        // way a typed edit does.
        const writeValue = (value) => {
          if (!valueInput || valueInput.value === value) return;
          valueInput.value = value;
          valueInput.dispatchEvent(new Event("input", { bubbles: true }));
          valueInput.dispatchEvent(new Event("change", { bubbles: true }));
        };
        // aria-selected follows the bound value, after a selection and after a
        // morph that re-renders the options.
        const markSelected = () => {
          const value = valueInput ? valueInput.value : null;
          for (const option of options()) {
            const chosen = value !== null && value !== "" && option.getAttribute("data-sn-value") === value;
            setAttribute(option, "aria-selected", String(chosen));
          }
        };
        const select = (option) => {
          writeValue(option.getAttribute("data-sn-value") ?? option.textContent.trim());
          markSelected();
          input.value = option.textContent.trim();
          input.dispatchEvent(new Event("input", { bubbles: true }));
          input.dispatchEvent(new Event("change", { bubbles: true }));
          selected = true;
          setExpanded(false);
        };
        input.addEventListener(
          "input",
          (event) => {
            // The selection's own label write arrives here too; only an edit
            // by the user unsets the value it chose.
            if (event.isTrusted) {
              writeValue("");
              markSelected();
            }
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
            markSelected();
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
        // A re-render that changes the bound value rewrites the attribute.
        if (valueInput) observe(valueInput, markSelected, { attributes: true, attributeFilter: ["value"] });
        markSelected();
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
