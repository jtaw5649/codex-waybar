project(
  'codex_shimmer', 'c',
  version: '0.1.0',
  license: 'MIT'
)

cc = meson.get_compiler('c')
deps = [
  dependency('gtk+-3.0', version: '>=3.22.0'),
  dependency('json-glib-1.0'),
  cc.find_library('m', required: false)
]

shared_library(
  'wb_codex_shimmer',
  ['main.c'],
  dependencies: deps,
  name_prefix: '',
  install: true,
  install_dir: join_paths(get_option('prefix'), 'lib', 'waybar')
)
