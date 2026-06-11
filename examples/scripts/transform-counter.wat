(module
  (memory (export "memory") 2)
  
  ;; process(input_ptr: i32, input_len: i32) -> i32
  ;; Identity: copies input to output at offset 65536 with length prefix
  (func (export "process") (param $input_ptr i32) (param $input_len i32) (result i32)
    ;; store the 4-byte LE length
    (i32.store (i32.const 65536) (local.get $input_len))
    ;; copy the input JSON body after the length prefix
    (memory.copy (i32.const 65540) (local.get $input_ptr) (local.get $input_len))
    ;; return pointer to output (offset 65536)
    (i32.const 65536)
  )
)
