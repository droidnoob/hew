; Methods (functions with a receiver)
(method_declaration
  name: (field_identifier) @name) @definition.method

; Top-level functions
(function_declaration
  name: (identifier) @name) @definition.function

; Type-as-class: struct declarations
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (struct_type))) @definition.class

; Interfaces
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (interface_type))) @definition.interface
