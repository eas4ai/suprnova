// The remote list belongs to both its text and its monotonic query sequence.
// No client substring filter is applied to fuzzy or synonym search results.
(function () {
  const acceptsResults = (currentQuery, resultQuery) =>
    typeof currentQuery === "string" &&
    typeof resultQuery === "string" &&
    currentQuery === resultQuery;
  class QuerySequence {
    #latest;
    constructor(initial = 0) {
      this.#latest = initial;
    }
    ask() {
      return ++this.#latest;
    }
    accepts(sequence) {
      return sequence === this.#latest;
    }
  }
  const api = Object.freeze({ QuerySequence, acceptsResults });
  if (
    typeof customElements === "undefined" ||
    customElements.get("sn-search-box")
  )
    return api;
  const attribute = (element, name, value) => {
    if (element.getAttribute(name) !== value) element.setAttribute(name, value);
  };
  const remove = (element, name) => {
    if (element.hasAttribute(name)) element.removeAttribute(name);
  };
  const hidden = (element, value) => {
    if (element.hidden !== value) element.hidden = value;
  };
  customElements.define(
    "sn-search-box",
    class extends HTMLElement {
      #connection = null;
      connectedCallback() {
        this.#connection?.abort();
        const input = this.querySelector("input[role=combobox]");
        const list = this.querySelector("[role=listbox]");
        if (!input || !list) return;
        const connection = new AbortController();
        this.#connection = connection;
        const { signal } = connection;
        const sequenceInput = this.querySelector("[data-sn-search-sequence]");
        const sequence = new QuerySequence(
          Number(sequenceInput?.value ?? list.dataset.snSequence ?? 0),
        );
        let active = -1;
        let selected = false;
        const options = () =>
          Array.from(list.querySelectorAll("[role=option]"));
        const close = () => {
          hidden(list, true);
          attribute(input, "aria-expanded", "false");
          remove(input, "aria-activedescendant");
          for (const option of options())
            attribute(option, "aria-selected", "false");
          active = -1;
        };
        const render = () => {
          const current =
            acceptsResults(input.value, list.dataset.snQuery ?? "") &&
            sequence.accepts(Number(list.dataset.snSequence ?? 0));
          if (
            !current ||
            selected ||
            document.activeElement !== input ||
            options().length === 0
          ) {
            close();
            return;
          }
          hidden(list, false);
          attribute(input, "aria-expanded", "true");
        };
        const activate = (position) => {
          const choices = options();
          if (list.hidden || choices.length === 0) return;
          active = (position + choices.length) % choices.length;
          choices.forEach((option, index) =>
            attribute(option, "aria-selected", String(index === active)),
          );
          attribute(input, "aria-activedescendant", choices[active].id);
          choices[active].scrollIntoView({ block: "nearest" });
        };
        const write = (selector, value) => {
          const field = this.querySelector(selector);
          if (!field) return;
          field.value = value;
          field.dispatchEvent(new Event("input", { bubbles: true }));
        };
        const select = (option) => {
          if (!option || list.hidden) return;
          write("[data-sn-search-selection]", option.dataset.snKey ?? "");
          write("[data-sn-search-position]", option.dataset.snPosition ?? "0");
          // The island's action records the click and emits search.selected only
          // after the server accepts the selected key and impression position.
          this.querySelector("[data-sn-search-select]")?.click();
          selected = true;
          close();
        };
        input.addEventListener(
          "input",
          () => {
            selected = false;
            write("[data-sn-search-sequence]", String(sequence.ask()));
            render();
          },
          { signal },
        );
        input.addEventListener(
          "focus",
          () => {
            selected = false;
            render();
          },
          { signal },
        );
        input.addEventListener("blur", close, { signal });
        input.addEventListener(
          "keydown",
          (event) => {
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              selected = false;
              render();
              const next =
                active < 0 && event.key === "ArrowUp"
                  ? options().length - 1
                  : active + (event.key === "ArrowDown" ? 1 : -1);
              activate(next);
            } else if (event.key === "Home" && !list.hidden) {
              event.preventDefault();
              activate(0);
            } else if (event.key === "End" && !list.hidden) {
              event.preventDefault();
              activate(options().length - 1);
            } else if (event.key === "Enter" && active >= 0 && !list.hidden) {
              event.preventDefault();
              select(options()[active]);
            } else if (event.key === "Escape") {
              event.preventDefault();
              selected = true;
              close();
            }
          },
          { signal },
        );
        list.addEventListener(
          "pointerdown",
          (event) => {
            const option =
              event.target instanceof Element
                ? event.target.closest("[role=option]")
                : null;
            if (option) {
              event.preventDefault();
              select(option);
            }
          },
          { signal },
        );
        const observer = new MutationObserver(() => {
          const previous = active;
          render();
          if (previous >= 0 && !list.hidden)
            activate(Math.min(previous, options().length - 1));
        });
        observer.observe(list, {
          childList: true,
          attributes: true,
          attributeFilter: ["data-sn-query", "data-sn-sequence", "hidden"],
        });
        signal.addEventListener("abort", () => observer.disconnect(), {
          once: true,
        });
        attribute(this, "data-sn-upgraded", "");
      }
      connectedMoveCallback() {}
      disconnectedCallback() {
        this.#connection?.abort();
        this.#connection = null;
      }
    },
  );
  return api;
})();
