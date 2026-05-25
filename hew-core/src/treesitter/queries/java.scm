; Methods inside a class
(class_declaration
  body: (class_body
    (method_declaration
      name: (identifier) @name) @definition.method))

; Classes
(class_declaration
  name: (identifier) @name) @definition.class

; Interfaces
(interface_declaration
  name: (identifier) @name) @definition.interface
