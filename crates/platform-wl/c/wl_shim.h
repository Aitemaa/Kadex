#ifndef KADEX_WL_SHIM_H
#define KADEX_WL_SHIM_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

typedef struct KdxApp KdxApp;
typedef struct KdxWin KdxWin;
typedef struct KdxLayout KdxLayout;

enum KdxWinKind {
    KDX_WIN_XDG = 0,
    KDX_WIN_LAYER_OVERLAY = 1,
};

enum KdxMod {
    KDX_MOD_SHIFT = 1 << 0,
    KDX_MOD_CTRL  = 1 << 1,
    KDX_MOD_ALT   = 1 << 2,
    KDX_MOD_SUPER = 1 << 3,
};

enum KdxCursor {
    KDX_CURSOR_DEFAULT = 0,
    KDX_CURSOR_TEXT    = 1,
    KDX_CURSOR_POINTER = 2,
};

typedef struct KdxCallbacks {
    void *userdata;
    void (*on_configure)(void *ud, uint64_t win_id, int32_t width, int32_t height, uint32_t scale_120);
    void (*on_close)(void *ud, uint64_t win_id);
    void (*on_focus)(void *ud, uint64_t win_id, bool focused);
    void (*on_key)(void *ud, uint64_t win_id, uint32_t keysym, uint32_t mods, const char *utf8, size_t utf8_len, bool pressed);
    void (*on_ime_commit)(void *ud, uint64_t win_id, const char *utf8, size_t utf8_len);
    void (*on_pointer_motion)(void *ud, uint64_t win_id, double x, double y);
    void (*on_pointer_button)(void *ud, uint64_t win_id, uint32_t button, bool pressed, uint32_t mods, double x, double y);
    void (*on_pointer_scroll)(void *ud, uint64_t win_id, double dx, double dy, uint32_t mods);
    void (*on_repeat_info)(void *ud, int32_t rate, int32_t delay_ms);
} KdxCallbacks;

// App lifecycle
KdxApp *kdx_app_new(const KdxCallbacks *cbs);
void kdx_app_destroy(KdxApp *app);
int kdx_app_get_fd(KdxApp *app);
int kdx_app_prepare_read(KdxApp *app);
int kdx_app_read_events(KdxApp *app);
void kdx_app_cancel_read(KdxApp *app);
int kdx_app_dispatch_pending(KdxApp *app);
int kdx_app_flush(KdxApp *app);
bool kdx_app_has_layer_shell(KdxApp *app);

// Window lifecycle
KdxWin *kdx_win_create(KdxApp *app, uint64_t win_id, int kind, const char *app_id, const char *title, int32_t width, int32_t height);
void kdx_win_destroy(KdxWin *win);
void kdx_win_set_title(KdxWin *win, const char *title);
void kdx_win_start_move(KdxWin *win);
void kdx_win_set_cursor(KdxApp *app, int cursor_kind);

// Clipboard & Primary Selection
void kdx_clipboard_set(KdxApp *app, const char *utf8, size_t len, bool primary);
// Caller frees returned buffer with kdx_free_bytes
char *kdx_clipboard_get(KdxApp *app, bool primary, size_t *out_len);
void kdx_free_bytes(char *ptr);

// Painting (Cairo on wl_shm buffer)
typedef struct KdxCanvas {
    void *cr; // cairo_t *
    int32_t width;
    int32_t height;
    uint32_t scale_120;
} KdxCanvas;

bool kdx_win_begin_paint(KdxWin *win, KdxCanvas *out_canvas);
void kdx_win_end_paint(KdxWin *win, KdxCanvas *canvas);

// Drawing primitives
void kdx_draw_rect(void *cr, double x, double y, double w, double h, uint32_t rgb, double alpha);
void kdx_draw_round_rect(void *cr, double x, double y, double w, double h, double r, uint32_t rgb, double alpha);
void kdx_draw_circle(void *cr, double cx, double cy, double r, uint32_t rgb, double alpha);
void kdx_draw_line(void *cr, double x1, double y1, double x2, double y2, double lw, uint32_t rgb, double alpha);
void kdx_clip_rect(void *cr, double x, double y, double w, double h);
void kdx_save(void *cr);
void kdx_restore(void *cr);
void kdx_translate(void *cr, double tx, double ty);
void kdx_scale(void *cr, double sx, double sy);
void kdx_stroke_round_rect(void *cr, double x, double y, double w, double h, double r, double lw, uint32_t rgb, double alpha);
void kdx_clip_round_rect(void *cr, double x, double y, double w, double h, double r);
void kdx_stroke_circle(void *cr, double cx, double cy, double r, double lw, uint32_t rgb, double alpha);
void kdx_path_begin(void *cr);
void kdx_path_move(void *cr, double x, double y);
void kdx_path_line(void *cr, double x, double y);
void kdx_path_curve(void *cr, double x1, double y1, double x2, double y2, double x3, double y3);
void kdx_path_close(void *cr);
void kdx_path_stroke(void *cr, double lw, uint32_t rgb, double alpha);
bool kdx_font_register(const char *path);

// Pango text layout helpers
KdxLayout *kdx_layout_new(void *cr, const char *font_family, double font_size_pt, const char *text, size_t text_len, int32_t wrap_width_px);
void kdx_layout_free(KdxLayout *layout);
void kdx_layout_set_subpixel(KdxLayout *layout);
void kdx_layout_get_size(KdxLayout *layout, double *out_w, double *out_h);
void kdx_layout_draw(void *cr, KdxLayout *layout, double x, double y, uint32_t rgb, double alpha);
void kdx_layout_index_to_pos(KdxLayout *layout, int byte_idx, double *out_x, double *out_y, double *out_h);
int kdx_layout_xy_to_index(KdxLayout *layout, double x, double y);
void kdx_layout_draw_selection(void *cr, KdxLayout *layout, double ox, double oy, int start_byte, int end_byte, uint32_t rgb, double alpha);
int kdx_hit_test(const char *font_family, double font_size_pt, const char *text, size_t text_len, int32_t wrap_width_px, double x, double y);
double kdx_measure_line_height(const char *font_family, double font_size_pt);

#endif
