(module
  (import "jaringan" "fetch" (func $fetch (param i32 i32) (result i32)))
  (import "jaringan" "log" (func $log (param i32 i32 i32 i32)))
  (memory (export "memory") 2)
  
  ;; process(input_ptr: i32, input_len: i32) -> i32
  ;; This is an identity transform that also demonstrates bridge import.
  ;; A real script would call $fetch to get external data.
  (func (export "process") (param $input_ptr i32) (param $input_len i32) (result i32)
    ;; Copy input JSON to output at offset 65536 with length prefix
    (i32.store (i32.const 65536) (local.get $input_len))
    (memory.copy (i32.const 65540) (local.get $input_ptr) (local.get $input_len))
    (i32.const 65536)
  )
)
