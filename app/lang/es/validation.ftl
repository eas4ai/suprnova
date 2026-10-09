### Dogfood app validation overrides - Spanish.
###
### The framework ships no Spanish validation catalog (only the embedded
### English one), so any key an app wants translated for `es` has to be
### defined here. `POST /lang-demo` trips `validation-required` on its
### `name` field - see `app/src/controllers/lang_demo.rs`.

validation-required = El campo { $field } es obligatorio.
