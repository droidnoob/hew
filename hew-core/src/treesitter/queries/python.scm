; Class methods (functions inside a class body)
(class_definition
  body: (block
    (function_definition
      name: (identifier) @name) @definition.method))

; Top-level functions
(function_definition
  name: (identifier) @name) @definition.function

; Classes
(class_definition
  name: (identifier) @name) @definition.class
