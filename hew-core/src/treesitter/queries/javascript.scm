; Class methods
(class_declaration
  body: (class_body
    (method_definition
      name: (property_identifier) @name) @definition.method))

; Top-level functions
(function_declaration
  name: (identifier) @name) @definition.function

; Classes
(class_declaration
  name: (identifier) @name) @definition.class
