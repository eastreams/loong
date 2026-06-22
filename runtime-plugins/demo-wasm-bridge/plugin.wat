;; Demo Loong WASM plugin using the loong_json_abi_v1 core WebAssembly ABI.
;; The host writes a JSON request into guest memory, calls loong_invoke(ptr, len),
;; and reads the JSON response from the returned packed pointer/length.
(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 2048))

  ;; Fixed demo response at memory offset 1024, length 101 bytes.
  ;; The payload includes an empty messages array so both send_message and
  ;; receive_batch demo paths can complete without external network services.
  (data (i32.const 1024) "{\"payload\":{\"ok\":true,\"via\":\"wasm\",\"messages\":[],\"demo_message\":\"handled by Loong demo WASM plugin\"}}")

  (func (export "loong_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (local.get $ptr)
  )

  (func (export "loong_free") (param $ptr i32) (param $len i32))

  (func (export "loong_invoke") (param $ptr i32) (param $len i32) (result i64)
    (i64.or
      (i64.shl (i64.extend_i32_u (i32.const 1024)) (i64.const 32))
      (i64.extend_i32_u (i32.const 101))
    )
  )
)
