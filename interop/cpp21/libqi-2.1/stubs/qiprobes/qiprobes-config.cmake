# No-op replacement of the LTTng probes (qiprobes) the 2014 qibuild projects look for.
set(_QIPROBES_STUB_INC "${CMAKE_CURRENT_LIST_DIR}/../inc")
macro(qiprobes_create_probe name)
  qi_submodule_create(${name} SRC "${_QIPROBES_STUB_INC}/tp_qi.h")
endmacro()
function(qiprobes_instrument_files)
endfunction()
include_directories("${_QIPROBES_STUB_INC}")
