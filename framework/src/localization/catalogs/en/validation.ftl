### Framework validation messages — English.
### Apps override any of these by defining the same id in any
### lang/<locale>/*.ftl file.
###
### $field is the (possibly translated) field name — define
### `field-<name>` in your own catalog to give a field a human label.
### The remaining parameters mirror the rule's constructor arguments.

validation-invalid-data = The given data was invalid.
validation-required = The { $field } field is required.
validation-email = The { $field } field must be a valid email address.
validation-min = The { $field } field must be at least { $min } characters.
validation-max = The { $field } field must be at most { $max } characters.
validation-between = The { $field } field must be between { $min } and { $max } characters.
validation-in = The selected { $field } is invalid.
validation-not-in = The selected { $field } is invalid.
validation-integer = The { $field } field must be an integer.
validation-numeric = The { $field } field must be a number.
validation-boolean = The { $field } field must be true or false.
validation-alpha = The { $field } field must contain only letters.
validation-alpha-num = The { $field } field must contain only letters and numbers.
validation-alpha-dash = The { $field } field must contain only letters, numbers, dashes, and underscores.
validation-url = The { $field } field must be a valid URL.
validation-http-url = The { $field } field must be a valid HTTP or HTTPS URL.
validation-uuid = The { $field } field must be a valid UUID.
validation-required-if = The { $field } field is required when { $other } is { $value }.
validation-required-with = The { $field } field is required when { $others } is present.
validation-required-with-all = The { $field } field is required when { $others } are present.
validation-required-unless = The { $field } field is required unless { $other } is { $value }.
validation-same = The { $field } field must match { $other }.
validation-different = The { $field } field must differ from { $other }.
validation-confirmed = The { $field } field confirmation does not match.
validation-unique = The { $field } has already been taken.
