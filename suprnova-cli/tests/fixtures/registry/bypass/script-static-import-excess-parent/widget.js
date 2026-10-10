// script-static-import-excess-parent: one `../` more than the component is deep leaves the components' root, so under a path prefix it resolves outside the prefix
import "../../../evil-ui/script-static-import-excess-parent/helper.js"; // refused: script-import
