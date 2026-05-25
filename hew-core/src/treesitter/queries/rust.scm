; Functions
(function_item
  name: (identifier) @name) @definition.function

; Impl methods
(impl_item
  body: (declaration_list
    (function_item
      name: (identifier) @name) @definition.method))

; Trait methods (signatures + provided defaults)
(trait_item
  body: (declaration_list
    (function_item
      name: (identifier) @name) @definition.method))

; Types-as-class: struct / enum
(struct_item
  name: (type_identifier) @name) @definition.class

(enum_item
  name: (type_identifier) @name) @definition.class

; Traits-as-interface
(trait_item
  name: (type_identifier) @name) @definition.interface

; Modules
(mod_item
  name: (identifier) @name) @definition.module
