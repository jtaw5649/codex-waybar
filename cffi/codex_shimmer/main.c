#include "waybar_cffi_module.h"

#include <cairo.h>
#include <gio/gio.h>
#include <gtk/gtk.h>
#include <json-glib/json-glib.h>
#include <math.h>
#include <pango/pangocairo.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
  wbcffi_module *module;
  GtkWidget *container;
  GtkWidget *drawing_area;

  gchar *cache_path;
  gchar *text_plain;
  gchar *tooltip_text;
  GPtrArray *style_classes;

  GdkRGBA base_rgba;
  GdkRGBA highlight_rgba;

  double period_ms;
  double width_chars;
  double cycles;
  guint tick_ms;
  double pause_ms;
  double highlight_alpha;
  double base_alpha;

  guint timeout_id;
  GFileMonitor *monitor;
  guint64 start_time_us;
} CodexShimmer;

static gboolean animation_tick(gpointer user_data);
static gboolean on_draw(GtkWidget *widget, cairo_t *cr, gpointer user_data);
static gboolean load_cache(CodexShimmer *inst);
static void handle_file_change(GFileMonitor *monitor, GFile *file, GFile *other_file,
                               GFileMonitorEvent event, gpointer user_data);
static void update_tooltip(CodexShimmer *inst);
static void free_style_classes(CodexShimmer *inst);
static void update_style_classes(CodexShimmer *inst, JsonArray *classes);

const size_t wbcffi_version = 2;

static gchar *default_cache_path(void) {
  const gchar *home = g_get_home_dir();
  return g_build_filename(home, ".cache", "codex-shimmer", "latest.json", NULL);
}

static JsonNode *parse_json_value(const char *value) {
  if (!value) {
    return NULL;
  }
  JsonParser *parser = json_parser_new();
  GError *error = NULL;
  if (!json_parser_load_from_data(parser, value, -1, &error)) {
    g_warning("codex-shimmer: failed to parse config value '%s': %s", value,
              error ? error->message : "unknown error");
    g_clear_error(&error);
    g_object_unref(parser);
    return NULL;
  }
  JsonNode *root = json_node_copy(json_parser_get_root(parser));
  g_object_unref(parser);
  return root;
}

static gchar *expand_user_path(const gchar *path) {
  if (!path) {
    return NULL;
  }
  if (path[0] == '~') {
    const gchar *home = g_get_home_dir();
    return g_build_filename(home, path + 1, NULL);
  }
  return g_strdup(path);
}

static gboolean json_node_is_number(JsonNode *node) {
  if (!node || json_node_get_node_type(node) != JSON_NODE_VALUE) {
    return FALSE;
  }
  GType value_type = json_node_get_value_type(node);
  return value_type == G_TYPE_DOUBLE || value_type == G_TYPE_FLOAT || value_type == G_TYPE_INT64 ||
         value_type == G_TYPE_INT || value_type == G_TYPE_UINT || value_type == G_TYPE_LONG ||
         value_type == G_TYPE_ULONG;
}

static double json_node_get_double_or(JsonNode *node, double fallback) {
  if (!json_node_is_number(node)) {
    return fallback;
  }
  return json_node_get_double(node);
}

static guint json_node_get_uint_or(JsonNode *node, guint fallback) {
  if (!json_node_is_number(node)) {
    return fallback;
  }
  return (guint)CLAMP(json_node_get_int(node), 0, G_MAXUINT);
}

static gboolean json_node_is_string(JsonNode *node) {
  if (!node || json_node_get_node_type(node) != JSON_NODE_VALUE) {
    return FALSE;
  }
  return json_node_get_value_type(node) == G_TYPE_STRING;
}

static const gchar *config_lookup(const wbcffi_config_entry *entries, size_t len, const char *key) {
  for (size_t i = 0; i < len; ++i) {
    if (g_strcmp0(entries[i].key, key) == 0) {
      return entries[i].value;
    }
  }
  return NULL;
}

static gboolean setup_monitor(CodexShimmer *inst) {
  if (inst->monitor) {
    g_object_unref(inst->monitor);
    inst->monitor = NULL;
  }
  GFile *file = g_file_new_for_path(inst->cache_path);
  GError *error = NULL;
  inst->monitor = g_file_monitor_file(file, G_FILE_MONITOR_NONE, NULL, &error);
  g_object_unref(file);
  if (!inst->monitor) {
    g_warning("codex-shimmer: unable to monitor %s: %s", inst->cache_path,
              error ? error->message : "unknown error");
    g_clear_error(&error);
    return FALSE;
  }
  g_signal_connect(inst->monitor, "changed", G_CALLBACK(handle_file_change), inst);
  return TRUE;
}

void *wbcffi_init(const wbcffi_init_info *init_info, const wbcffi_config_entry *config_entries,
                  size_t config_entries_len) {
  CodexShimmer *inst = g_new0(CodexShimmer, 1);
  inst->module = init_info->obj;
  inst->period_ms = 1600.0;
  inst->width_chars = 4.0;
  inst->cycles = 1.0;
  inst->tick_ms = 33;
  inst->pause_ms = 500.0;
  inst->highlight_alpha = 0.35;
  inst->base_alpha = 1.0;
  inst->style_classes = g_ptr_array_new_with_free_func(g_free);
  gdk_rgba_parse(&inst->base_rgba, "#C7D3FF");
  gdk_rgba_parse(&inst->highlight_rgba, "#FFFFFF");
  inst->highlight_rgba.alpha = inst->highlight_alpha;

  const gchar *cfg;
  JsonNode *node;

  cfg = config_lookup(config_entries, config_entries_len, "cache_path");
  if (cfg) {
    node = parse_json_value(cfg);
    if (json_node_is_string(node)) {
      gchar *tmp = expand_user_path(json_node_get_string(node));
      if (tmp) {
        inst->cache_path = tmp;
      }
    }
    if (node)
      json_node_free(node);
  }
  if (!inst->cache_path)
    inst->cache_path = default_cache_path();

  cfg = config_lookup(config_entries, config_entries_len, "period_ms");
  if (cfg) {
    node = parse_json_value(cfg);
    inst->period_ms = json_node_get_double_or(node, inst->period_ms);
    if (node)
      json_node_free(node);
  }

  cfg = config_lookup(config_entries, config_entries_len, "width_chars");
  if (cfg) {
    node = parse_json_value(cfg);
    inst->width_chars = json_node_get_double_or(node, inst->width_chars);
    if (node)
      json_node_free(node);
  } else {
    cfg = config_lookup(config_entries, config_entries_len, "width");
    if (cfg) {
      node = parse_json_value(cfg);
      inst->width_chars = json_node_get_double_or(node, inst->width_chars);
      if (node)
        json_node_free(node);
    }
  }

  cfg = config_lookup(config_entries, config_entries_len, "pause_ms");
  if (cfg) {
    node = parse_json_value(cfg);
    inst->pause_ms = json_node_get_double_or(node, inst->pause_ms);
    if (node)
      json_node_free(node);
  }

  cfg = config_lookup(config_entries, config_entries_len, "cycles");
  if (cfg) {
    node = parse_json_value(cfg);
    inst->cycles = json_node_get_double_or(node, inst->cycles);
    if (node)
      json_node_free(node);
  }

  cfg = config_lookup(config_entries, config_entries_len, "tick_ms");
  if (cfg) {
    node = parse_json_value(cfg);
    inst->tick_ms = json_node_get_uint_or(node, inst->tick_ms);
    if (node)
      json_node_free(node);
  }

  cfg = config_lookup(config_entries, config_entries_len, "base_color");
  if (cfg) {
    node = parse_json_value(cfg);
    if (json_node_is_string(node)) {
      gdk_rgba_parse(&inst->base_rgba, json_node_get_string(node));
    }
    if (node)
      json_node_free(node);
  }

  cfg = config_lookup(config_entries, config_entries_len, "highlight_color");
  if (cfg) {
    node = parse_json_value(cfg);
    if (json_node_is_string(node)) {
      gdk_rgba_parse(&inst->highlight_rgba, json_node_get_string(node));
    }
    if (node)
      json_node_free(node);
  }

  cfg = config_lookup(config_entries, config_entries_len, "highlight_alpha");
  if (cfg) {
    node = parse_json_value(cfg);
    inst->highlight_alpha = CLAMP(json_node_get_double_or(node, inst->highlight_alpha), 0.0, 1.0);
    if (node)
      json_node_free(node);
  }

  cfg = config_lookup(config_entries, config_entries_len, "base_alpha");
  if (cfg) {
    node = parse_json_value(cfg);
    inst->base_alpha = CLAMP(json_node_get_double_or(node, inst->base_alpha), 0.0, 1.0);
    if (node)
      json_node_free(node);
  }

  inst->period_ms = MAX(inst->period_ms, 200.0);
  inst->width_chars = CLAMP(inst->width_chars, 1.0, 20.0);
  inst->cycles = CLAMP(inst->cycles, 0.1, 6.0);
  inst->tick_ms = CLAMP(inst->tick_ms, 5, 1000);
  inst->pause_ms = MAX(inst->pause_ms, 0.0);
  inst->highlight_rgba.alpha = inst->highlight_alpha;
  inst->base_rgba.alpha = inst->base_alpha;

  GtkContainer *root = init_info->get_root_widget(init_info->obj);
  inst->container = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
  gtk_widget_set_name(inst->container, "codex-shimmer");
  gtk_container_add(root, inst->container);

  inst->drawing_area = gtk_drawing_area_new();
  gtk_widget_set_hexpand(inst->drawing_area, TRUE);
  gtk_widget_set_vexpand(inst->drawing_area, FALSE);
  gtk_widget_set_halign(inst->drawing_area, GTK_ALIGN_FILL);
  gtk_widget_set_valign(inst->drawing_area, GTK_ALIGN_CENTER);
  g_signal_connect(inst->drawing_area, "draw", G_CALLBACK(on_draw), inst);
  gtk_box_pack_start(GTK_BOX(inst->container), inst->drawing_area, TRUE, TRUE, 0);

  gtk_widget_show_all(inst->container);

  inst->start_time_us = g_get_monotonic_time();

  setup_monitor(inst);
  load_cache(inst);

  inst->timeout_id = g_timeout_add(inst->tick_ms, animation_tick, inst);

  return inst;
}

void wbcffi_deinit(void *instance) {
  CodexShimmer *inst = instance;
  if (!inst)
    return;
  if (inst->timeout_id)
    g_source_remove(inst->timeout_id);
  if (inst->monitor)
    g_object_unref(inst->monitor);
  g_clear_pointer(&inst->cache_path, g_free);
  g_clear_pointer(&inst->text_plain, g_free);
  g_clear_pointer(&inst->tooltip_text, g_free);
  free_style_classes(inst);
  g_ptr_array_unref(inst->style_classes);
  g_free(inst);
}

void wbcffi_update(void *instance) {
  CodexShimmer *inst = instance;
  if (!inst)
    return;
  gtk_widget_queue_draw(inst->drawing_area);
}

void wbcffi_refresh(void *instance, int signal) {
  (void)signal;
  CodexShimmer *inst = instance;
  if (!inst)
    return;
  load_cache(inst);
}

void wbcffi_doaction(void *instance, const char *name) {
  if (!instance || !name)
    return;
  if (g_strcmp0(name, "reload") == 0) {
    wbcffi_refresh(instance, 0);
  }
}

static gboolean animation_tick(gpointer user_data) {
  CodexShimmer *inst = user_data;
  gtk_widget_queue_draw(inst->drawing_area);
  return G_SOURCE_CONTINUE;
}

static gboolean on_draw(GtkWidget *widget, cairo_t *cr, gpointer user_data) {
  CodexShimmer *inst = user_data;
  if (!inst->text_plain)
    return FALSE;

  GtkAllocation alloc;
  gtk_widget_get_allocation(widget, &alloc);

  PangoLayout *layout = gtk_widget_create_pango_layout(widget, inst->text_plain);
  PangoAttrList *attrs = pango_attr_list_new();
  PangoAttribute *bold = pango_attr_weight_new(PANGO_WEIGHT_BOLD);
  bold->start_index = 0;
  bold->end_index = G_MAXUINT;
  pango_attr_list_insert(attrs, bold);
  pango_layout_set_attributes(layout, attrs);
  pango_attr_list_unref(attrs);

  int layout_width_px = 0, layout_height_px = 0;
  pango_layout_get_pixel_size(layout, &layout_width_px, &layout_height_px);
  if (layout_width_px <= 0) {
    g_object_unref(layout);
    return TRUE;
  }

  double x = 0.0;
  double y = (alloc.height - layout_height_px) / 2.0;

  cairo_save(cr);
  cairo_translate(cr, x, y);
  cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
  gdk_cairo_set_source_rgba(cr, &inst->base_rgba);
  pango_cairo_show_layout(cr, layout);
  cairo_restore(cr);

  double elapsed_ms = (g_get_monotonic_time() - inst->start_time_us) / 1000.0;
  double total_cycle = inst->period_ms + inst->pause_ms;
  double cycle_pos = fmod(elapsed_ms, total_cycle);
  if (cycle_pos >= inst->period_ms || inst->highlight_alpha <= 0.0) {
    g_object_unref(layout);
    return TRUE;
  }

  int glyphs = MAX(1, g_utf8_strlen(inst->text_plain, -1));
  double avg_char_width = glyphs > 0 ? layout_width_px / (double)glyphs : layout_width_px;
  double base_width = MAX(avg_char_width * inst->width_chars, avg_char_width);

  double phase = cycle_pos / inst->period_ms;
  double envelope = phase < 0.5 ? phase * 2.0 : (1.0 - phase) * 2.0;
  double width_px = MAX(avg_char_width * 0.6, base_width * envelope);
  width_px = MAX(width_px, avg_char_width * 0.6);

  double start_offset = -width_px;
  double travel = layout_width_px + width_px * 2.0;
  double center_px = start_offset + phase * travel;

  double gradient_start = center_px - width_px * 2.0;
  double gradient_end = center_px + width_px * 2.0;
  if (gradient_end <= gradient_start) {
    gradient_end = gradient_start + 1.0;
  }

  cairo_pattern_t *pattern = cairo_pattern_create_linear(gradient_start, 0.0, gradient_end, 0.0);
  int steps = 96;
  for (int i = 0; i <= steps; ++i) {
    double offset = (double)i / steps;
    double px = gradient_start + offset * (gradient_end - gradient_start);
    double delta = (px - center_px) / width_px;
    double gaussian = exp(-0.5 * delta * delta);
    double alpha = inst->highlight_alpha * envelope * gaussian;
    alpha = CLAMP(alpha, 0.0, 1.0);
    cairo_pattern_add_color_stop_rgba(pattern, offset, inst->highlight_rgba.red,
                                      inst->highlight_rgba.green, inst->highlight_rgba.blue,
                                      alpha);
  }

  cairo_save(cr);
  cairo_translate(cr, x, y);
  pango_cairo_layout_path(cr, layout);
  cairo_clip(cr);
  cairo_set_operator(cr, CAIRO_OPERATOR_SCREEN);
  cairo_set_source(cr, pattern);
  cairo_paint(cr);
  cairo_restore(cr);
  cairo_pattern_destroy(pattern);

  g_object_unref(layout);
  return TRUE;
}

static void handle_file_change(GFileMonitor *monitor, GFile *file, GFile *other_file,
                               GFileMonitorEvent event, gpointer user_data) {
  (void)monitor;
  (void)file;
  (void)other_file;
  CodexShimmer *inst = user_data;
  switch (event) {
  case G_FILE_MONITOR_EVENT_CHANGED:
  case G_FILE_MONITOR_EVENT_CREATED:
  case G_FILE_MONITOR_EVENT_CHANGES_DONE_HINT:
  case G_FILE_MONITOR_EVENT_MOVED_IN:
  case G_FILE_MONITOR_EVENT_MOVED:
    load_cache(inst);
    break;
  default:
    break;
  }
}

static void update_tooltip(CodexShimmer *inst) {
  gtk_widget_set_tooltip_text(inst->drawing_area, inst->tooltip_text);
}

static void free_style_classes(CodexShimmer *inst) {
  GtkStyleContext *ctx = gtk_widget_get_style_context(inst->container);
  for (guint i = 0; i < inst->style_classes->len; ++i) {
    const gchar *cl = g_ptr_array_index(inst->style_classes, i);
    gtk_style_context_remove_class(ctx, cl);
  }
  g_ptr_array_set_size(inst->style_classes, 0);
}

static void update_style_classes(CodexShimmer *inst, JsonArray *classes) {
  free_style_classes(inst);
  if (!classes)
    return;
  GtkStyleContext *ctx = gtk_widget_get_style_context(inst->container);
  guint len = json_array_get_length(classes);
  for (guint i = 0; i < len; ++i) {
    JsonNode *node = json_array_get_element(classes, i);
    if (!json_node_is_string(node))
      continue;
    const gchar *cl = json_node_get_string(node);
    gtk_style_context_add_class(ctx, cl);
    g_ptr_array_add(inst->style_classes, g_strdup(cl));
  }
}

static gboolean load_cache(CodexShimmer *inst) {
  gchar *contents = NULL;
  gsize length = 0;
  if (!g_file_get_contents(inst->cache_path, &contents, &length, NULL)) {
    g_message("codex-shimmer: unable to read %s", inst->cache_path);
    return FALSE;
  }

  JsonParser *parser = json_parser_new();
  GError *error = NULL;
  gboolean ok = json_parser_load_from_data(parser, contents, length, &error);
  g_free(contents);
  if (!ok) {
    g_warning("codex-shimmer: failed to parse %s: %s", inst->cache_path,
              error ? error->message : "unknown error");
    g_clear_error(&error);
    g_object_unref(parser);
    return FALSE;
  }

  JsonNode *root = json_parser_get_root(parser);
  if (!JSON_NODE_HOLDS_OBJECT(root)) {
    g_object_unref(parser);
    return FALSE;
  }

  JsonObject *obj = json_node_get_object(root);
  const gchar *text = json_object_has_member(obj, "text") ? json_object_get_string_member(obj, "text") : NULL;
  const gchar *tooltip = json_object_has_member(obj, "tooltip") ?
                             json_object_get_string_member(obj, "tooltip") : NULL;
  JsonNode *classes_node = json_object_has_member(obj, "class") ?
                               json_object_get_member(obj, "class") : NULL;
  JsonArray *classes = (classes_node && json_node_get_node_type(classes_node) == JSON_NODE_ARRAY)
                           ? json_node_get_array(classes_node)
                           : NULL;

  if (!classes && classes_node && json_node_is_string(classes_node)) {
    classes = json_array_new();
    json_array_add_string_element(classes, json_node_get_string(classes_node));
  }

  g_free(inst->text_plain);
  inst->text_plain = text ? g_strdup(text) : g_strdup("Waiting for Codex…");

  g_free(inst->tooltip_text);
  inst->tooltip_text = tooltip ? g_strdup(tooltip) : NULL;
  update_tooltip(inst);

  update_style_classes(inst, classes);

  PangoLayout *layout = gtk_widget_create_pango_layout(inst->drawing_area, inst->text_plain);
  int width_px = 0, height_px = 0;
  pango_layout_get_pixel_size(layout, &width_px, &height_px);
  gtk_widget_set_size_request(inst->drawing_area, MAX(width_px + 16, 80), height_px + 8);
  g_object_unref(layout);
  gtk_widget_queue_resize(inst->drawing_area);

  g_message("codex-shimmer: refreshed text '%s' (width=%d)", inst->text_plain, width_px);

  if (classes && classes_node && json_node_is_string(classes_node)) {
    json_array_unref(classes);
  }

  inst->start_time_us = g_get_monotonic_time();
  gtk_widget_queue_draw(inst->drawing_area);

  g_object_unref(parser);
  return TRUE;
}
