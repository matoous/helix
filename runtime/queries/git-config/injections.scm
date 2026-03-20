((comment) @injection.content
 (#set! injection.language "comment"))

; https://git-scm.com/docs/gitattributes#_defining_a_custom_hunk_header
; https://git-scm.com/docs/gitattributes#_customizing_word_diff
; e.g.
; ```
; [diff "tex"]
; 	xfuncname = "^(\\\\(sub)*section\\{.*)$"
; 	wordRegex = "\\\\[a-zA-Z]+|[{}]|\\\\.|[^\\{}[:space:]]+"
; ```
(variable
 (name) @_var (#any-of? @_var "xfuncname" "wordRegex")
 value: (string) @injection.content
 (#set! injection.language "regex"))

((section
  (section_header
   (section_name) @_section)
  (variable
   value: (string) @injection.content))
 (#eq? @_section "alias")
 (#set! injection.language "sh"))

((section
  (section_header
   (section_name) @_section)
  (variable
   (name) @_name
   value: (string) @injection.content))
 (#eq? @_section "credential")
 (#eq? @_name "helper")
 (#set! injection.language "sh"))
