#include "wl_shim.h"

#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <poll.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include <wayland-client.h>
#include <wayland-cursor.h>
#include <xkbcommon/xkbcommon.h>
#include <cairo/cairo.h>
#include <pango/pangocairo.h>
#include <pango/pangofc-fontmap.h>
#include <fontconfig/fontconfig.h>

#include "xdg-shell-client-protocol.h"
#include "xdg-decoration-unstable-v1-client-protocol.h"
#include "fractional-scale-v1-client-protocol.h"
#include "viewporter-client-protocol.h"
#include "wp-primary-selection-unstable-v1-client-protocol.h"
#include "text-input-unstable-v3-client-protocol.h"
#include "wlr-layer-shell-unstable-v1-client-protocol.h"

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

typedef struct ShmBuf {
    struct wl_buffer *buffer;
    void *data;
    size_t size;
    int32_t width;
    int32_t height;
    bool busy;
} ShmBuf;

struct KdxWin {
    KdxApp *app;
    uint64_t id;
    int kind;
    int32_t width;
    int32_t height;
    uint32_t scale_120;
    bool configured;

    struct wl_surface *surface;
    struct xdg_surface *xdg_surface;
    struct xdg_toplevel *xdg_toplevel;
    struct zxdg_toplevel_decoration_v1 *decoration;
    struct zwlr_layer_surface_v1 *layer_surface;
    struct wp_fractional_scale_v1 *frac_scale;
    struct wp_viewport *viewport;

    ShmBuf bufs[2];
    int active_buf;
    cairo_surface_t *cairo_surf;

    struct KdxWin *next;
};

struct KdxApp {
    KdxCallbacks cbs;

    struct wl_display *display;
    struct wl_registry *registry;
    struct wl_compositor *compositor;
    struct wl_shm *shm;
    struct wl_seat *seat;
    struct wl_keyboard *keyboard;
    struct wl_pointer *pointer;
    struct wl_data_device_manager *data_dev_mgr;
    struct wl_data_device *data_device;
    struct xdg_wm_base *xdg_wm_base;
    struct zxdg_decoration_manager_v1 *deco_mgr;
    struct wp_fractional_scale_manager_v1 *frac_mgr;
    struct wp_viewporter *viewporter;
    struct zwp_primary_selection_device_manager_v1 *prim_mgr;
    struct zwp_primary_selection_device_v1 *prim_device;
    struct zwp_text_input_manager_v3 *ti_mgr;
    struct zwp_text_input_v3 *text_input;
    struct zwlr_layer_shell_v1 *layer_shell;

    // Cursor
    struct wl_cursor_theme *cursor_theme;
    struct wl_surface *cursor_surface;
    uint32_t pointer_enter_serial;
    int cur_cursor_kind;

    // XKB
    struct xkb_context *xkb_ctx;
    struct xkb_keymap *xkb_keymap;
    struct xkb_state *xkb_state;
    uint32_t mods;

    // Focus & pointer state
    KdxWin *kbd_focus;
    KdxWin *ptr_focus;
    double ptr_x;
    double ptr_y;
    uint32_t last_serial;
    uint32_t last_button_serial;

    // Clipboard
    struct wl_data_offer *clip_offer;
    char *clip_offer_mime;
    struct wl_data_source *clip_source;
    char *clip_owned_data;
    size_t clip_owned_len;

    // Primary selection
    struct zwp_primary_selection_offer_v1 *prim_offer;
    char *prim_offer_mime;
    struct zwp_primary_selection_source_v1 *prim_source;
    char *prim_owned_data;
    size_t prim_owned_len;

    // IME pending commit
    char *ime_pending_commit;

    KdxWin *windows;
};

struct KdxLayout {
    PangoLayout *layout;
};

static bool app_owns_win(KdxApp *app, KdxWin *target) {
    if (!target) return false;
    for (KdxWin *w = app->windows; w; w = w->next) {
        if (w == target) return true;
    }
    return false;
}

static KdxWin *win_from_surface(KdxApp *app, struct wl_surface *surf) {
    if (!surf) return NULL;
    for (KdxWin *w = app->windows; w; w = w->next) {
        if (w->surface == surf) return w;
    }
    return NULL;
}

static void shm_buf_destroy(ShmBuf *b) {
    if (b->buffer) {
        wl_buffer_destroy(b->buffer);
        b->buffer = NULL;
    }
    if (b->data && b->size > 0) {
        munmap(b->data, b->size);
        b->data = NULL;
        b->size = 0;
    }
    b->width = 0;
    b->height = 0;
    b->busy = false;
}

static void wl_buf_release(void *data, struct wl_buffer *buf) {
    (void)buf;
    ShmBuf *b = (ShmBuf *)data;
    b->busy = false;
}

static const struct wl_buffer_listener wl_buf_listener = {
    .release = wl_buf_release,
};

static bool shm_buf_ensure(KdxApp *app, ShmBuf *b, int32_t w, int32_t h) {
    if (w < 1) w = 1;
    if (h < 1) h = 1;
    if (b->buffer && b->width == w && b->height == h) {
        return true;
    }
    shm_buf_destroy(b);

    int32_t stride = w * 4;
    size_t size = (size_t)stride * (size_t)h;
    int fd = memfd_create("kadex-shm", MFD_CLOEXEC);
    if (fd < 0) return false;
    if (ftruncate(fd, (off_t)size) < 0) {
        close(fd);
        return false;
    }
    void *data = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (data == MAP_FAILED) {
        close(fd);
        return false;
    }
    struct wl_shm_pool *pool = wl_shm_create_pool(app->shm, fd, (int32_t)size);
    close(fd);
    if (!pool) {
        munmap(data, size);
        return false;
    }
    struct wl_buffer *buffer = wl_shm_pool_create_buffer(
        pool, 0, w, h, stride, WL_SHM_FORMAT_ARGB8888
    );
    wl_shm_pool_destroy(pool);
    if (!buffer) {
        munmap(data, size);
        return false;
    }
    wl_buffer_add_listener(buffer, &wl_buf_listener, b);
    b->buffer = buffer;
    b->data = data;
    b->size = size;
    b->width = w;
    b->height = h;
    b->busy = false;
    return true;
}

// ---- XDG WM Base ----
static void xdg_wm_base_ping(void *data, struct xdg_wm_base *wm, uint32_t serial) {
    (void)data;
    xdg_wm_base_pong(wm, serial);
}

static const struct xdg_wm_base_listener xdg_wm_base_listener = {
    .ping = xdg_wm_base_ping,
};

// ---- Keyboard ----
static void update_mods(KdxApp *app) {
    if (!app->xkb_state) {
        app->mods = 0;
        return;
    }
    uint32_t m = 0;
    if (xkb_state_mod_name_is_active(app->xkb_state, XKB_MOD_NAME_SHIFT, XKB_STATE_MODS_EFFECTIVE) > 0)
        m |= KDX_MOD_SHIFT;
    if (xkb_state_mod_name_is_active(app->xkb_state, XKB_MOD_NAME_CTRL, XKB_STATE_MODS_EFFECTIVE) > 0)
        m |= KDX_MOD_CTRL;
    if (xkb_state_mod_name_is_active(app->xkb_state, XKB_MOD_NAME_ALT, XKB_STATE_MODS_EFFECTIVE) > 0)
        m |= KDX_MOD_ALT;
    if (xkb_state_mod_name_is_active(app->xkb_state, XKB_MOD_NAME_LOGO, XKB_STATE_MODS_EFFECTIVE) > 0)
        m |= KDX_MOD_SUPER;
    app->mods = m;
}

static void kbd_keymap(void *data, struct wl_keyboard *kbd, uint32_t format, int32_t fd, uint32_t size) {
    (void)kbd;
    KdxApp *app = (KdxApp *)data;
    if (format != WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1) {
        close(fd);
        return;
    }
    char *map = mmap(NULL, size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (map == MAP_FAILED) return;

    struct xkb_keymap *km = xkb_keymap_new_from_string(
        app->xkb_ctx, map, XKB_KEYMAP_FORMAT_TEXT_V1, XKB_KEYMAP_COMPILE_NO_FLAGS
    );
    munmap(map, size);
    if (!km) return;

    struct xkb_state *st = xkb_state_new(km);
    if (!st) {
        xkb_keymap_unref(km);
        return;
    }
    if (app->xkb_state) xkb_state_unref(app->xkb_state);
    if (app->xkb_keymap) xkb_keymap_unref(app->xkb_keymap);
    app->xkb_keymap = km;
    app->xkb_state = st;
    update_mods(app);
}

static void kbd_enter(void *data, struct wl_keyboard *kbd, uint32_t serial,
                      struct wl_surface *surface, struct wl_array *keys) {
    (void)kbd;
    (void)keys;
    KdxApp *app = (KdxApp *)data;
    app->last_serial = serial;
    KdxWin *w = win_from_surface(app, surface);
    app->kbd_focus = w;
    if (app->text_input && w) {
        zwp_text_input_v3_enable(app->text_input);
        zwp_text_input_v3_commit(app->text_input);
    }
    if (w && app->cbs.on_focus) {
        app->cbs.on_focus(app->cbs.userdata, w->id, true);
    }
}

static void kbd_leave(void *data, struct wl_keyboard *kbd, uint32_t serial, struct wl_surface *surface) {
    (void)kbd;
    (void)serial;
    KdxApp *app = (KdxApp *)data;
    KdxWin *w = win_from_surface(app, surface);
    if (app->text_input) {
        zwp_text_input_v3_disable(app->text_input);
        zwp_text_input_v3_commit(app->text_input);
    }
    if (app->kbd_focus == w) {
        app->kbd_focus = NULL;
    }
    if (w && app->cbs.on_focus) {
        app->cbs.on_focus(app->cbs.userdata, w->id, false);
    }
}

static void kbd_key(void *data, struct wl_keyboard *kbd, uint32_t serial, uint32_t time,
                    uint32_t key, uint32_t state) {
    (void)kbd;
    (void)time;
    KdxApp *app = (KdxApp *)data;
    app->last_serial = serial;
    if (!app_owns_win(app, app->kbd_focus) || !app->xkb_state) return;

    uint32_t keycode = key + 8;
    xkb_keysym_t sym = xkb_state_key_get_one_sym(app->xkb_state, keycode);
    char utf8[64];
    int len = 0;
    if (state == WL_KEYBOARD_KEY_STATE_PRESSED) {
        len = xkb_state_key_get_utf8(app->xkb_state, keycode, utf8, sizeof(utf8));
        if (len < 0) len = 0;
    }
    if (app->cbs.on_key) {
        app->cbs.on_key(
            app->cbs.userdata,
            app->kbd_focus->id,
            (uint32_t)sym,
            app->mods,
            utf8,
            (size_t)len,
            state == WL_KEYBOARD_KEY_STATE_PRESSED
        );
    }
}

static void kbd_modifiers(void *data, struct wl_keyboard *kbd, uint32_t serial,
                          uint32_t mods_depressed, uint32_t mods_latched,
                          uint32_t mods_locked, uint32_t group) {
    (void)kbd;
    (void)serial;
    KdxApp *app = (KdxApp *)data;
    if (!app->xkb_state) return;
    xkb_state_update_mask(app->xkb_state, mods_depressed, mods_latched, mods_locked, 0, 0, group);
    update_mods(app);
}

static void kbd_repeat_info(void *data, struct wl_keyboard *kbd, int32_t rate, int32_t delay) {
    (void)kbd;
    KdxApp *app = (KdxApp *)data;
    if (app->cbs.on_repeat_info) {
        app->cbs.on_repeat_info(app->cbs.userdata, rate, delay);
    }
}

static const struct wl_keyboard_listener kbd_listener = {
    .keymap = kbd_keymap,
    .enter = kbd_enter,
    .leave = kbd_leave,
    .key = kbd_key,
    .modifiers = kbd_modifiers,
    .repeat_info = kbd_repeat_info,
};

// ---- Pointer ----
static void apply_cursor(KdxApp *app, int kind) {
    if (!app->pointer || !app->cursor_theme || !app->cursor_surface) return;
    const char *name = "left_ptr";
    if (kind == KDX_CURSOR_TEXT) name = "xterm";
    else if (kind == KDX_CURSOR_POINTER) name = "hand2";

    struct wl_cursor *cursor = wl_cursor_theme_get_cursor(app->cursor_theme, name);
    if (!cursor && kind == KDX_CURSOR_TEXT)
        cursor = wl_cursor_theme_get_cursor(app->cursor_theme, "text");
    if (!cursor && kind == KDX_CURSOR_POINTER)
        cursor = wl_cursor_theme_get_cursor(app->cursor_theme, "pointer");
    if (!cursor)
        cursor = wl_cursor_theme_get_cursor(app->cursor_theme, "left_ptr");
    if (!cursor || cursor->image_count == 0) return;

    struct wl_cursor_image *img = cursor->images[0];
    struct wl_buffer *buf = wl_cursor_image_get_buffer(img);
    if (!buf) return;
    wl_pointer_set_cursor(
        app->pointer, app->pointer_enter_serial, app->cursor_surface,
        (int32_t)img->hotspot_x, (int32_t)img->hotspot_y
    );
    wl_surface_attach(app->cursor_surface, buf, 0, 0);
    wl_surface_damage(app->cursor_surface, 0, 0, (int32_t)img->width, (int32_t)img->height);
    wl_surface_commit(app->cursor_surface);
}

void kdx_win_set_cursor(KdxApp *app, int cursor_kind) {
    if (!app || app->cur_cursor_kind == cursor_kind) return;
    app->cur_cursor_kind = cursor_kind;
    apply_cursor(app, cursor_kind);
}

static void ptr_enter(void *data, struct wl_pointer *ptr, uint32_t serial,
                      struct wl_surface *surface, wl_fixed_t sx, wl_fixed_t sy) {
    (void)ptr;
    KdxApp *app = (KdxApp *)data;
    app->pointer_enter_serial = serial;
    app->last_serial = serial;
    app->ptr_focus = win_from_surface(app, surface);
    app->ptr_x = wl_fixed_to_double(sx);
    app->ptr_y = wl_fixed_to_double(sy);
    apply_cursor(app, app->cur_cursor_kind);
}

static void ptr_leave(void *data, struct wl_pointer *ptr, uint32_t serial, struct wl_surface *surface) {
    (void)ptr;
    (void)serial;
    KdxApp *app = (KdxApp *)data;
    if (app->ptr_focus == win_from_surface(app, surface)) {
        // Tell the view the pointer left (it clears hover states); -1e6 is the "left" sentinel.
        if (app_owns_win(app, app->ptr_focus) && app->cbs.on_pointer_motion) {
            app->cbs.on_pointer_motion(app->cbs.userdata, app->ptr_focus->id, -1.0e6, -1.0e6);
        }
        app->ptr_focus = NULL;
    }
}

static void ptr_motion(void *data, struct wl_pointer *ptr, uint32_t time, wl_fixed_t sx, wl_fixed_t sy) {
    (void)ptr;
    (void)time;
    KdxApp *app = (KdxApp *)data;
    app->ptr_x = wl_fixed_to_double(sx);
    app->ptr_y = wl_fixed_to_double(sy);
    if (app_owns_win(app, app->ptr_focus) && app->cbs.on_pointer_motion) {
        app->cbs.on_pointer_motion(app->cbs.userdata, app->ptr_focus->id, app->ptr_x, app->ptr_y);
    }
}

static void ptr_button(void *data, struct wl_pointer *ptr, uint32_t serial, uint32_t time,
                       uint32_t button, uint32_t state) {
    (void)ptr;
    (void)time;
    KdxApp *app = (KdxApp *)data;
    app->last_serial = serial;
    if (state == WL_POINTER_BUTTON_STATE_PRESSED) {
        app->last_button_serial = serial;
    }
    if (app_owns_win(app, app->ptr_focus) && app->cbs.on_pointer_button) {
        app->cbs.on_pointer_button(
            app->cbs.userdata,
            app->ptr_focus->id,
            button,
            state == WL_POINTER_BUTTON_STATE_PRESSED,
            app->mods,
            app->ptr_x,
            app->ptr_y
        );
    }
}

static void ptr_axis(void *data, struct wl_pointer *ptr, uint32_t time, uint32_t axis, wl_fixed_t value) {
    (void)ptr;
    (void)time;
    KdxApp *app = (KdxApp *)data;
    if (!app_owns_win(app, app->ptr_focus) || !app->cbs.on_pointer_scroll) return;
    double v = wl_fixed_to_double(value);
    double dx = (axis == WL_POINTER_AXIS_HORIZONTAL_SCROLL) ? v : 0.0;
    double dy = (axis == WL_POINTER_AXIS_VERTICAL_SCROLL) ? v : 0.0;
    app->cbs.on_pointer_scroll(app->cbs.userdata, app->ptr_focus->id, dx, dy, app->mods);
}

static void ptr_frame(void *d, struct wl_pointer *p) { (void)d; (void)p; }
static void ptr_axis_source(void *d, struct wl_pointer *p, uint32_t s) { (void)d; (void)p; (void)s; }
static void ptr_axis_stop(void *d, struct wl_pointer *p, uint32_t t, uint32_t a) { (void)d; (void)p; (void)t; (void)a; }
static void ptr_axis_discrete(void *d, struct wl_pointer *p, uint32_t a, int32_t s) { (void)d; (void)p; (void)a; (void)s; }

static const struct wl_pointer_listener ptr_listener = {
    .enter = ptr_enter,
    .leave = ptr_leave,
    .motion = ptr_motion,
    .button = ptr_button,
    .axis = ptr_axis,
    .frame = ptr_frame,
    .axis_source = ptr_axis_source,
    .axis_stop = ptr_axis_stop,
    .axis_discrete = ptr_axis_discrete,
};

// ---- Clipboard (wl_data_device) ----
static bool is_text_mime(const char *mime) {
    return strcmp(mime, "text/plain;charset=utf-8") == 0
        || strcmp(mime, "UTF8_STRING") == 0
        || strcmp(mime, "text/plain") == 0;
}

static void data_offer_offer(void *data, struct wl_data_offer *offer, const char *mime) {
    (void)offer;
    char **best_mime = (char **)data;
    if (!is_text_mime(mime)) return;
    if (!*best_mime || strcmp(mime, "text/plain;charset=utf-8") == 0) {
        free(*best_mime);
        *best_mime = strdup(mime);
    }
}
static void data_offer_source_actions(void *d, struct wl_data_offer *o, uint32_t a) { (void)d; (void)o; (void)a; }
static void data_offer_action(void *d, struct wl_data_offer *o, uint32_t a) { (void)d; (void)o; (void)a; }

static const struct wl_data_offer_listener data_offer_listener = {
    .offer = data_offer_offer,
    .source_actions = data_offer_source_actions,
    .action = data_offer_action,
};

static void data_dev_data_offer(void *data, struct wl_data_device *dev, struct wl_data_offer *offer) {
    (void)data;
    (void)dev;
    char **mime_slot = (char **)calloc(1, sizeof(char *));
    wl_data_offer_add_listener(offer, &data_offer_listener, mime_slot);
}

static void data_dev_enter(void *d, struct wl_data_device *dv, uint32_t s, struct wl_surface *sf, wl_fixed_t x, wl_fixed_t y, struct wl_data_offer *o) {
    (void)d; (void)dv; (void)s; (void)sf; (void)x; (void)y;
    // Drag-and-drop isn't supported: release the offer and its mime slot instead of leaking them.
    if (o) {
        char **mime_slot = (char **)wl_data_offer_get_user_data(o);
        if (mime_slot) {
            free(*mime_slot);
            free(mime_slot);
        }
        wl_data_offer_destroy(o);
    }
}
static void data_dev_leave(void *d, struct wl_data_device *dv) { (void)d; (void)dv; }
static void data_dev_motion(void *d, struct wl_data_device *dv, uint32_t t, wl_fixed_t x, wl_fixed_t y) { (void)d; (void)dv; (void)t; (void)x; (void)y; }
static void data_dev_drop(void *d, struct wl_data_device *dv) { (void)d; (void)dv; }

static void data_dev_selection(void *data, struct wl_data_device *dev, struct wl_data_offer *offer) {
    (void)dev;
    KdxApp *app = (KdxApp *)data;
    if (app->clip_offer) {
        wl_data_offer_destroy(app->clip_offer);
        app->clip_offer = NULL;
    }
    free(app->clip_offer_mime);
    app->clip_offer_mime = NULL;

    if (offer) {
        char **mime_slot = (char **)wl_data_offer_get_user_data(offer);
        app->clip_offer = offer;
        if (mime_slot) {
            app->clip_offer_mime = *mime_slot;
            free(mime_slot);
        }
    }
}

static const struct wl_data_device_listener data_dev_listener = {
    .data_offer = data_dev_data_offer,
    .enter = data_dev_enter,
    .leave = data_dev_leave,
    .motion = data_dev_motion,
    .drop = data_dev_drop,
    .selection = data_dev_selection,
};

static void data_src_target(void *d, struct wl_data_source *s, const char *m) { (void)d; (void)s; (void)m; }
static void data_src_send(void *data, struct wl_data_source *src, const char *mime, int32_t fd) {
    (void)src;
    (void)mime;
    KdxApp *app = (KdxApp *)data;
    if (app->clip_owned_data && app->clip_owned_len > 0) {
        size_t off = 0;
        while (off < app->clip_owned_len) {
            ssize_t n = write(fd, app->clip_owned_data + off, app->clip_owned_len - off);
            if (n <= 0) break;
            off += (size_t)n;
        }
    }
    close(fd);
}
static void data_src_cancelled(void *data, struct wl_data_source *src) {
    KdxApp *app = (KdxApp *)data;
    if (app->clip_source == src) {
        app->clip_source = NULL;
        free(app->clip_owned_data);
        app->clip_owned_data = NULL;
        app->clip_owned_len = 0;
    }
    wl_data_source_destroy(src);
}
static void data_src_dnd_drop(void *d, struct wl_data_source *s) { (void)d; (void)s; }
static void data_src_dnd_fin(void *d, struct wl_data_source *s) { (void)d; (void)s; }
static void data_src_action(void *d, struct wl_data_source *s, uint32_t a) { (void)d; (void)s; (void)a; }

static const struct wl_data_source_listener data_src_listener = {
    .target = data_src_target,
    .send = data_src_send,
    .cancelled = data_src_cancelled,
    .dnd_drop_performed = data_src_dnd_drop,
    .dnd_finished = data_src_dnd_fin,
    .action = data_src_action,
};

// ---- Primary selection (middle-click paste) ----
static void prim_offer_offer(void *data, struct zwp_primary_selection_offer_v1 *offer, const char *mime) {
    (void)offer;
    char **best_mime = (char **)data;
    if (!is_text_mime(mime)) return;
    if (!*best_mime || strcmp(mime, "text/plain;charset=utf-8") == 0) {
        free(*best_mime);
        *best_mime = strdup(mime);
    }
}

static const struct zwp_primary_selection_offer_v1_listener prim_offer_listener = {
    .offer = prim_offer_offer,
};

static void prim_dev_data_offer(void *data, struct zwp_primary_selection_device_v1 *dev,
                                struct zwp_primary_selection_offer_v1 *offer) {
    (void)data;
    (void)dev;
    char **mime_slot = (char **)calloc(1, sizeof(char *));
    zwp_primary_selection_offer_v1_add_listener(offer, &prim_offer_listener, mime_slot);
}

static void prim_dev_selection(void *data, struct zwp_primary_selection_device_v1 *dev,
                               struct zwp_primary_selection_offer_v1 *offer) {
    (void)dev;
    KdxApp *app = (KdxApp *)data;
    if (app->prim_offer) {
        zwp_primary_selection_offer_v1_destroy(app->prim_offer);
        app->prim_offer = NULL;
    }
    free(app->prim_offer_mime);
    app->prim_offer_mime = NULL;

    if (offer) {
        char **mime_slot = (char **)zwp_primary_selection_offer_v1_get_user_data(offer);
        app->prim_offer = offer;
        if (mime_slot) {
            app->prim_offer_mime = *mime_slot;
            free(mime_slot);
        }
    }
}

static const struct zwp_primary_selection_device_v1_listener prim_dev_listener = {
    .data_offer = prim_dev_data_offer,
    .selection = prim_dev_selection,
};

static void prim_src_send(void *data, struct zwp_primary_selection_source_v1 *src, const char *mime, int32_t fd) {
    (void)src;
    (void)mime;
    KdxApp *app = (KdxApp *)data;
    if (app->prim_owned_data && app->prim_owned_len > 0) {
        size_t off = 0;
        while (off < app->prim_owned_len) {
            ssize_t n = write(fd, app->prim_owned_data + off, app->prim_owned_len - off);
            if (n <= 0) break;
            off += (size_t)n;
        }
    }
    close(fd);
}

static void prim_src_cancelled(void *data, struct zwp_primary_selection_source_v1 *src) {
    KdxApp *app = (KdxApp *)data;
    if (app->prim_source == src) {
        app->prim_source = NULL;
        free(app->prim_owned_data);
        app->prim_owned_data = NULL;
        app->prim_owned_len = 0;
    }
    zwp_primary_selection_source_v1_destroy(src);
}

static const struct zwp_primary_selection_source_v1_listener prim_src_listener = {
    .send = prim_src_send,
    .cancelled = prim_src_cancelled,
};

void kdx_clipboard_set(KdxApp *app, const char *utf8, size_t len, bool primary) {
    if (!app || !utf8) return;
    if (!primary) {
        if (!app->data_dev_mgr || !app->data_device) return;
        free(app->clip_owned_data);
        app->clip_owned_data = (char *)malloc(len + 1);
        if (!app->clip_owned_data) return;
        memcpy(app->clip_owned_data, utf8, len);
        app->clip_owned_data[len] = '\0';
        app->clip_owned_len = len;

        if (app->clip_source) {
            wl_data_source_destroy(app->clip_source);
            app->clip_source = NULL;
        }
        app->clip_source = wl_data_device_manager_create_data_source(app->data_dev_mgr);
        wl_data_source_add_listener(app->clip_source, &data_src_listener, app);
        wl_data_source_offer(app->clip_source, "text/plain;charset=utf-8");
        wl_data_source_offer(app->clip_source, "UTF8_STRING");
        wl_data_source_offer(app->clip_source, "text/plain");
        wl_data_device_set_selection(app->data_device, app->clip_source, app->last_serial);
        wl_display_flush(app->display);
    } else {
        if (!app->prim_mgr || !app->prim_device) return;
        free(app->prim_owned_data);
        app->prim_owned_data = (char *)malloc(len + 1);
        if (!app->prim_owned_data) return;
        memcpy(app->prim_owned_data, utf8, len);
        app->prim_owned_data[len] = '\0';
        app->prim_owned_len = len;

        if (app->prim_source) {
            zwp_primary_selection_source_v1_destroy(app->prim_source);
            app->prim_source = NULL;
        }
        app->prim_source = zwp_primary_selection_device_manager_v1_create_source(app->prim_mgr);
        zwp_primary_selection_source_v1_add_listener(app->prim_source, &prim_src_listener, app);
        zwp_primary_selection_source_v1_offer(app->prim_source, "text/plain;charset=utf-8");
        zwp_primary_selection_source_v1_offer(app->prim_source, "UTF8_STRING");
        zwp_primary_selection_source_v1_offer(app->prim_source, "text/plain");
        zwp_primary_selection_device_v1_set_selection(app->prim_device, app->prim_source, app->last_serial);
        wl_display_flush(app->display);
    }
}

char *kdx_clipboard_get(KdxApp *app, bool primary, size_t *out_len) {
    if (out_len) *out_len = 0;
    if (!app) return NULL;

    // Fast path: self-owned selection avoids any pipe roundtrip
    if (!primary && app->clip_source && app->clip_owned_data) {
        char *copy = (char *)malloc(app->clip_owned_len + 1);
        if (!copy) return NULL;
        memcpy(copy, app->clip_owned_data, app->clip_owned_len);
        copy[app->clip_owned_len] = '\0';
        if (out_len) *out_len = app->clip_owned_len;
        return copy;
    }
    if (primary && app->prim_source && app->prim_owned_data) {
        char *copy = (char *)malloc(app->prim_owned_len + 1);
        if (!copy) return NULL;
        memcpy(copy, app->prim_owned_data, app->prim_owned_len);
        copy[app->prim_owned_len] = '\0';
        if (out_len) *out_len = app->prim_owned_len;
        return copy;
    }

    int fds[2];
    if (pipe2(fds, O_CLOEXEC) < 0) return NULL;

    if (!primary) {
        if (!app->clip_offer || !app->clip_offer_mime) {
            close(fds[0]);
            close(fds[1]);
            return NULL;
        }
        wl_data_offer_receive(app->clip_offer, app->clip_offer_mime, fds[1]);
    } else {
        if (!app->prim_offer || !app->prim_offer_mime) {
            close(fds[0]);
            close(fds[1]);
            return NULL;
        }
        zwp_primary_selection_offer_v1_receive(app->prim_offer, app->prim_offer_mime, fds[1]);
    }
    close(fds[1]);
    wl_display_flush(app->display);

    size_t cap = 4096;
    size_t len = 0;
    char *buf = (char *)malloc(cap);
    if (!buf) {
        close(fds[0]);
        return NULL;
    }

    struct pollfd pfd = { .fd = fds[0], .events = POLLIN, .revents = 0 };
    for (;;) {
        int r = poll(&pfd, 1, 250);
        if (r <= 0) break;
        if (len + 4096 > cap) {
            if (cap >= 16 * 1024 * 1024) break; // 16 MB safety cap
            cap *= 2;
            char *nb = (char *)realloc(buf, cap);
            if (!nb) break;
            buf = nb;
        }
        ssize_t n = read(fds[0], buf + len, cap - len - 1);
        if (n <= 0) break;
        len += (size_t)n;
    }
    close(fds[0]);
    buf[len] = '\0';
    if (out_len) *out_len = len;
    return buf;
}

void kdx_free_bytes(char *ptr) {
    free(ptr);
}

// ---- IME (zwp_text_input_v3) ----
static void ti_enter(void *d, struct zwp_text_input_v3 *ti, struct wl_surface *sf) { (void)d; (void)ti; (void)sf; }
static void ti_leave(void *d, struct zwp_text_input_v3 *ti, struct wl_surface *sf) { (void)d; (void)ti; (void)sf; }
static void ti_preedit(void *d, struct zwp_text_input_v3 *ti, const char *t, int32_t cb, int32_t ce) { (void)d; (void)ti; (void)t; (void)cb; (void)ce; }
static void ti_commit_string(void *data, struct zwp_text_input_v3 *ti, const char *text) {
    (void)ti;
    KdxApp *app = (KdxApp *)data;
    free(app->ime_pending_commit);
    app->ime_pending_commit = text ? strdup(text) : NULL;
}
static void ti_delete_surrounding(void *d, struct zwp_text_input_v3 *ti, uint32_t b, uint32_t a) { (void)d; (void)ti; (void)b; (void)a; }
static void ti_done(void *data, struct zwp_text_input_v3 *ti, uint32_t serial) {
    (void)ti;
    (void)serial;
    KdxApp *app = (KdxApp *)data;
    if (app->ime_pending_commit && app_owns_win(app, app->kbd_focus) && app->cbs.on_ime_commit) {
        app->cbs.on_ime_commit(
            app->cbs.userdata,
            app->kbd_focus->id,
            app->ime_pending_commit,
            strlen(app->ime_pending_commit)
        );
    }
    free(app->ime_pending_commit);
    app->ime_pending_commit = NULL;
}

static const struct zwp_text_input_v3_listener ti_listener = {
    .enter = ti_enter,
    .leave = ti_leave,
    .preedit_string = ti_preedit,
    .commit_string = ti_commit_string,
    .delete_surrounding_text = ti_delete_surrounding,
    .done = ti_done,
};

// ---- Seat ----
static void seat_capabilities(void *data, struct wl_seat *seat, uint32_t caps) {
    KdxApp *app = (KdxApp *)data;
    if ((caps & WL_SEAT_CAPABILITY_KEYBOARD) && !app->keyboard) {
        app->keyboard = wl_seat_get_keyboard(seat);
        wl_keyboard_add_listener(app->keyboard, &kbd_listener, app);
    } else if (!(caps & WL_SEAT_CAPABILITY_KEYBOARD) && app->keyboard) {
        wl_keyboard_destroy(app->keyboard);
        app->keyboard = NULL;
    }

    if ((caps & WL_SEAT_CAPABILITY_POINTER) && !app->pointer) {
        app->pointer = wl_seat_get_pointer(seat);
        wl_pointer_add_listener(app->pointer, &ptr_listener, app);
    } else if (!(caps & WL_SEAT_CAPABILITY_POINTER) && app->pointer) {
        wl_pointer_destroy(app->pointer);
        app->pointer = NULL;
    }
}

static void seat_name(void *d, struct wl_seat *s, const char *n) { (void)d; (void)s; (void)n; }

static const struct wl_seat_listener seat_listener = {
    .capabilities = seat_capabilities,
    .name = seat_name,
};

// ---- Registry ----
static void reg_global(void *data, struct wl_registry *reg, uint32_t name,
                       const char *iface, uint32_t version) {
    KdxApp *app = (KdxApp *)data;
    if (strcmp(iface, wl_compositor_interface.name) == 0) {
        uint32_t v = version < 4 ? version : 4;
        app->compositor = wl_registry_bind(reg, name, &wl_compositor_interface, v);
    } else if (strcmp(iface, wl_shm_interface.name) == 0) {
        app->shm = wl_registry_bind(reg, name, &wl_shm_interface, 1);
    } else if (strcmp(iface, wl_seat_interface.name) == 0) {
        uint32_t v = version < 5 ? version : 5;
        app->seat = wl_registry_bind(reg, name, &wl_seat_interface, v);
        wl_seat_add_listener(app->seat, &seat_listener, app);
    } else if (strcmp(iface, wl_data_device_manager_interface.name) == 0) {
        uint32_t v = version < 3 ? version : 3;
        app->data_dev_mgr = wl_registry_bind(reg, name, &wl_data_device_manager_interface, v);
    } else if (strcmp(iface, xdg_wm_base_interface.name) == 0) {
        uint32_t v = version < 2 ? version : 2;
        app->xdg_wm_base = wl_registry_bind(reg, name, &xdg_wm_base_interface, v);
        xdg_wm_base_add_listener(app->xdg_wm_base, &xdg_wm_base_listener, app);
    } else if (strcmp(iface, zxdg_decoration_manager_v1_interface.name) == 0) {
        app->deco_mgr = wl_registry_bind(reg, name, &zxdg_decoration_manager_v1_interface, 1);
    } else if (strcmp(iface, wp_fractional_scale_manager_v1_interface.name) == 0) {
        app->frac_mgr = wl_registry_bind(reg, name, &wp_fractional_scale_manager_v1_interface, 1);
    } else if (strcmp(iface, wp_viewporter_interface.name) == 0) {
        app->viewporter = wl_registry_bind(reg, name, &wp_viewporter_interface, 1);
    } else if (strcmp(iface, zwp_primary_selection_device_manager_v1_interface.name) == 0) {
        app->prim_mgr = wl_registry_bind(reg, name, &zwp_primary_selection_device_manager_v1_interface, 1);
    } else if (strcmp(iface, zwp_text_input_manager_v3_interface.name) == 0) {
        app->ti_mgr = wl_registry_bind(reg, name, &zwp_text_input_manager_v3_interface, 1);
    } else if (strcmp(iface, zwlr_layer_shell_v1_interface.name) == 0) {
        uint32_t v = version < 4 ? version : 4;
        app->layer_shell = wl_registry_bind(reg, name, &zwlr_layer_shell_v1_interface, v);
    }
}

static void reg_global_remove(void *d, struct wl_registry *r, uint32_t n) { (void)d; (void)r; (void)n; }

static const struct wl_registry_listener reg_listener = {
    .global = reg_global,
    .global_remove = reg_global_remove,
};

KdxApp *kdx_app_new(const KdxCallbacks *cbs) {
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) return NULL;

    KdxApp *app = (KdxApp *)calloc(1, sizeof(KdxApp));
    if (!app) {
        wl_display_disconnect(display);
        return NULL;
    }
    app->display = display;
    if (cbs) app->cbs = *cbs;
    app->xkb_ctx = xkb_context_new(XKB_CONTEXT_NO_FLAGS);

    app->registry = wl_display_get_registry(display);
    wl_registry_add_listener(app->registry, &reg_listener, app);
    wl_display_roundtrip(display);
    wl_display_roundtrip(display);

    if (!app->compositor || !app->shm || !app->xdg_wm_base) {
        kdx_app_destroy(app);
        return NULL;
    }

    if (app->seat && app->data_dev_mgr) {
        app->data_device = wl_data_device_manager_get_data_device(app->data_dev_mgr, app->seat);
        wl_data_device_add_listener(app->data_device, &data_dev_listener, app);
    }
    if (app->seat && app->prim_mgr) {
        app->prim_device = zwp_primary_selection_device_manager_v1_get_device(app->prim_mgr, app->seat);
        zwp_primary_selection_device_v1_add_listener(app->prim_device, &prim_dev_listener, app);
    }
    if (app->seat && app->ti_mgr) {
        app->text_input = zwp_text_input_manager_v3_get_text_input(app->ti_mgr, app->seat);
        zwp_text_input_v3_add_listener(app->text_input, &ti_listener, app);
    }

    app->cursor_theme = wl_cursor_theme_load(NULL, 24, app->shm);
    app->cursor_surface = wl_compositor_create_surface(app->compositor);
    app->cur_cursor_kind = KDX_CURSOR_DEFAULT;

    wl_display_roundtrip(display);
    return app;
}

void kdx_app_destroy(KdxApp *app) {
    if (!app) return;
    while (app->windows) {
        kdx_win_destroy(app->windows);
    }
    if (app->cursor_surface) wl_surface_destroy(app->cursor_surface);
    if (app->cursor_theme) wl_cursor_theme_destroy(app->cursor_theme);
    if (app->text_input) zwp_text_input_v3_destroy(app->text_input);
    if (app->ti_mgr) zwp_text_input_manager_v3_destroy(app->ti_mgr);
    if (app->prim_offer) zwp_primary_selection_offer_v1_destroy(app->prim_offer);
    if (app->prim_source) zwp_primary_selection_source_v1_destroy(app->prim_source);
    if (app->prim_device) zwp_primary_selection_device_v1_destroy(app->prim_device);
    if (app->prim_mgr) zwp_primary_selection_device_manager_v1_destroy(app->prim_mgr);
    if (app->clip_offer) wl_data_offer_destroy(app->clip_offer);
    if (app->clip_source) wl_data_source_destroy(app->clip_source);
    if (app->data_device) wl_data_device_destroy(app->data_device);
    if (app->data_dev_mgr) wl_data_device_manager_destroy(app->data_dev_mgr);
    if (app->layer_shell) zwlr_layer_shell_v1_destroy(app->layer_shell);
    if (app->frac_mgr) wp_fractional_scale_manager_v1_destroy(app->frac_mgr);
    if (app->viewporter) wp_viewporter_destroy(app->viewporter);
    if (app->deco_mgr) zxdg_decoration_manager_v1_destroy(app->deco_mgr);
    if (app->xdg_wm_base) xdg_wm_base_destroy(app->xdg_wm_base);
    if (app->keyboard) wl_keyboard_destroy(app->keyboard);
    if (app->pointer) wl_pointer_destroy(app->pointer);
    if (app->seat) wl_seat_destroy(app->seat);
    if (app->shm) wl_shm_destroy(app->shm);
    if (app->compositor) wl_compositor_destroy(app->compositor);
    if (app->registry) wl_registry_destroy(app->registry);
    if (app->xkb_state) xkb_state_unref(app->xkb_state);
    if (app->xkb_keymap) xkb_keymap_unref(app->xkb_keymap);
    if (app->xkb_ctx) xkb_context_unref(app->xkb_ctx);
    free(app->clip_offer_mime);
    free(app->clip_owned_data);
    free(app->prim_offer_mime);
    free(app->prim_owned_data);
    free(app->ime_pending_commit);
    if (app->display) wl_display_disconnect(app->display);
    free(app);
}

int kdx_app_get_fd(KdxApp *app) { return wl_display_get_fd(app->display); }
int kdx_app_prepare_read(KdxApp *app) { return wl_display_prepare_read(app->display); }
int kdx_app_read_events(KdxApp *app) { return wl_display_read_events(app->display); }
void kdx_app_cancel_read(KdxApp *app) { wl_display_cancel_read(app->display); }
int kdx_app_dispatch_pending(KdxApp *app) { return wl_display_dispatch_pending(app->display); }
int kdx_app_flush(KdxApp *app) { return wl_display_flush(app->display); }
bool kdx_app_has_layer_shell(KdxApp *app) { return app && app->layer_shell != NULL; }

// ---- Window Listeners ----
static void xdg_surf_configure(void *data, struct xdg_surface *surf, uint32_t serial) {
    KdxWin *w = (KdxWin *)data;
    xdg_surface_ack_configure(surf, serial);
    w->configured = true;
    if (w->app->cbs.on_configure) {
        w->app->cbs.on_configure(w->app->cbs.userdata, w->id, w->width, w->height, w->scale_120);
    }
}

static const struct xdg_surface_listener xdg_surf_listener = {
    .configure = xdg_surf_configure,
};

static void xdg_top_configure(void *data, struct xdg_toplevel *top, int32_t width, int32_t height, struct wl_array *states) {
    (void)top;
    (void)states;
    KdxWin *w = (KdxWin *)data;
    if (width > 0) w->width = width;
    if (height > 0) w->height = height;
}

static void xdg_top_close(void *data, struct xdg_toplevel *top) {
    (void)top;
    KdxWin *w = (KdxWin *)data;
    if (w->app->cbs.on_close) {
        w->app->cbs.on_close(w->app->cbs.userdata, w->id);
    }
}

static const struct xdg_toplevel_listener xdg_top_listener = {
    .configure = xdg_top_configure,
    .close = xdg_top_close,
};

static void deco_configure(void *d, struct zxdg_toplevel_decoration_v1 *dc, uint32_t m) {
    (void)d; (void)dc; (void)m;
}
static const struct zxdg_toplevel_decoration_v1_listener deco_listener = {
    .configure = deco_configure,
};

static void layer_configure(void *data, struct zwlr_layer_surface_v1 *ls, uint32_t serial, uint32_t width, uint32_t height) {
    KdxWin *w = (KdxWin *)data;
    zwlr_layer_surface_v1_ack_configure(ls, serial);
    if (width > 0) w->width = (int32_t)width;
    if (height > 0) w->height = (int32_t)height;
    w->configured = true;
    if (w->app->cbs.on_configure) {
        w->app->cbs.on_configure(w->app->cbs.userdata, w->id, w->width, w->height, w->scale_120);
    }
}

static void layer_closed(void *data, struct zwlr_layer_surface_v1 *ls) {
    (void)ls;
    KdxWin *w = (KdxWin *)data;
    if (w->app->cbs.on_close) {
        w->app->cbs.on_close(w->app->cbs.userdata, w->id);
    }
}

static const struct zwlr_layer_surface_v1_listener layer_listener = {
    .configure = layer_configure,
    .closed = layer_closed,
};

static void frac_preferred_scale(void *data, struct wp_fractional_scale_v1 *fs, uint32_t scale) {
    (void)fs;
    KdxWin *w = (KdxWin *)data;
    if (scale >= 60 && scale <= 600 && scale != w->scale_120) {
        w->scale_120 = scale;
        if (w->configured && w->app->cbs.on_configure) {
            w->app->cbs.on_configure(w->app->cbs.userdata, w->id, w->width, w->height, w->scale_120);
        }
    }
}

static const struct wp_fractional_scale_v1_listener frac_listener = {
    .preferred_scale = frac_preferred_scale,
};

KdxWin *kdx_win_create(KdxApp *app, uint64_t win_id, int kind, const char *app_id,
                       const char *title, int32_t width, int32_t height) {
    if (!app || !app->compositor) return NULL;
    KdxWin *w = (KdxWin *)calloc(1, sizeof(KdxWin));
    if (!w) return NULL;

    w->app = app;
    w->id = win_id;
    w->kind = kind;
    w->width = width > 0 ? width : 360;
    w->height = height > 0 ? height : 320;
    w->scale_120 = 120;
    w->active_buf = -1;

    w->surface = wl_compositor_create_surface(app->compositor);
    if (!w->surface) {
        free(w);
        return NULL;
    }

    if (app->frac_mgr) {
        w->frac_scale = wp_fractional_scale_manager_v1_get_fractional_scale(app->frac_mgr, w->surface);
        wp_fractional_scale_v1_add_listener(w->frac_scale, &frac_listener, w);
    }
    if (app->viewporter) {
        w->viewport = wp_viewporter_get_viewport(app->viewporter, w->surface);
    }

    if (kind == KDX_WIN_LAYER_OVERLAY && app->layer_shell) {
        w->layer_surface = zwlr_layer_shell_v1_get_layer_surface(
            app->layer_shell,
            w->surface,
            NULL,
            ZWLR_LAYER_SHELL_V1_LAYER_OVERLAY,
            app_id ? app_id : "kadex.overlay"
        );
        zwlr_layer_surface_v1_set_size(w->layer_surface, (uint32_t)w->width, (uint32_t)w->height);
        zwlr_layer_surface_v1_set_anchor(w->layer_surface, 0); // centered
        zwlr_layer_surface_v1_set_keyboard_interactivity(
            w->layer_surface, ZWLR_LAYER_SURFACE_V1_KEYBOARD_INTERACTIVITY_EXCLUSIVE
        );
        zwlr_layer_surface_v1_add_listener(w->layer_surface, &layer_listener, w);
    } else {
        w->kind = KDX_WIN_XDG;
        w->xdg_surface = xdg_wm_base_get_xdg_surface(app->xdg_wm_base, w->surface);
        xdg_surface_add_listener(w->xdg_surface, &xdg_surf_listener, w);
        w->xdg_toplevel = xdg_surface_get_toplevel(w->xdg_surface);
        xdg_toplevel_add_listener(w->xdg_toplevel, &xdg_top_listener, w);
        if (app_id) xdg_toplevel_set_app_id(w->xdg_toplevel, app_id);
        if (title) xdg_toplevel_set_title(w->xdg_toplevel, title);
        xdg_toplevel_set_min_size(w->xdg_toplevel, 220, 160);
        if (app->deco_mgr) {
            w->decoration = zxdg_decoration_manager_v1_get_toplevel_decoration(app->deco_mgr, w->xdg_toplevel);
            zxdg_toplevel_decoration_v1_add_listener(w->decoration, &deco_listener, w);
            zxdg_toplevel_decoration_v1_set_mode(w->decoration, ZXDG_TOPLEVEL_DECORATION_V1_MODE_SERVER_SIDE);
        }
    }

    w->next = app->windows;
    app->windows = w;

    wl_surface_commit(w->surface);
    wl_display_flush(app->display);
    return w;
}

void kdx_win_destroy(KdxWin *win) {
    if (!win) return;
    KdxApp *app = win->app;
    if (app) {
        if (app->kbd_focus == win) app->kbd_focus = NULL;
        if (app->ptr_focus == win) app->ptr_focus = NULL;
        KdxWin **pp = &app->windows;
        while (*pp) {
            if (*pp == win) {
                *pp = win->next;
                break;
            }
            pp = &(*pp)->next;
        }
    }
    if (win->cairo_surf) {
        cairo_surface_destroy(win->cairo_surf);
        win->cairo_surf = NULL;
    }
    shm_buf_destroy(&win->bufs[0]);
    shm_buf_destroy(&win->bufs[1]);
    if (win->decoration) zxdg_toplevel_decoration_v1_destroy(win->decoration);
    if (win->xdg_toplevel) xdg_toplevel_destroy(win->xdg_toplevel);
    if (win->xdg_surface) xdg_surface_destroy(win->xdg_surface);
    if (win->layer_surface) zwlr_layer_surface_v1_destroy(win->layer_surface);
    if (win->frac_scale) wp_fractional_scale_v1_destroy(win->frac_scale);
    if (win->viewport) wp_viewport_destroy(win->viewport);
    if (win->surface) wl_surface_destroy(win->surface);
    if (app && app->display) wl_display_flush(app->display);
    free(win);
}

void kdx_win_set_title(KdxWin *win, const char *title) {
    if (win && win->xdg_toplevel && title) {
        xdg_toplevel_set_title(win->xdg_toplevel, title);
    }
}

void kdx_win_start_move(KdxWin *win) {
    if (win && win->xdg_toplevel && win->app->seat) {
        xdg_toplevel_move(win->xdg_toplevel, win->app->seat, win->app->last_button_serial);
    }
}

// ---- Painting ----
bool kdx_win_begin_paint(KdxWin *win, KdxCanvas *out_canvas) {
    if (!win || !win->configured || !out_canvas) return false;
    int idx = !win->bufs[0].busy ? 0 : (!win->bufs[1].busy ? 1 : -1);
    // Both buffers are still held by the compositor: drawing now would overwrite pixels it may
    // be reading (torn / garbled frames under fast typing). Skip; the caller keeps needs_redraw
    // set and the wl_buffer.release event wakes the loop to retry.
    if (idx < 0) return false;

    int32_t phys_w, phys_h;
    if (win->viewport) {
        phys_w = (int32_t)(((int64_t)win->width * win->scale_120 + 60) / 120);
        phys_h = (int32_t)(((int64_t)win->height * win->scale_120 + 60) / 120);
    } else {
        int32_t iscale = (int32_t)((win->scale_120 + 60) / 120);
        if (iscale < 1) iscale = 1;
        phys_w = win->width * iscale;
        phys_h = win->height * iscale;
    }
    if (phys_w < 1) phys_w = 1;
    if (phys_h < 1) phys_h = 1;

    if (!shm_buf_ensure(win->app, &win->bufs[idx], phys_w, phys_h)) {
        return false;
    }

    ShmBuf *b = &win->bufs[idx];
    cairo_surface_t *surf = cairo_image_surface_create_for_data(
        (unsigned char *)b->data, CAIRO_FORMAT_ARGB32, phys_w, phys_h, phys_w * 4
    );
    cairo_t *cr = cairo_create(surf);
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_set_source_rgba(cr, 0, 0, 0, 0);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);

    double sx = (double)phys_w / (double)win->width;
    double sy = (double)phys_h / (double)win->height;
    cairo_scale(cr, sx, sy);

    win->active_buf = idx;
    win->cairo_surf = surf;
    out_canvas->cr = cr;
    out_canvas->width = win->width;
    out_canvas->height = win->height;
    out_canvas->scale_120 = win->scale_120;
    return true;
}

void kdx_win_end_paint(KdxWin *win, KdxCanvas *canvas) {
    if (!win || !canvas || !canvas->cr || win->active_buf < 0) return;
    cairo_t *cr = (cairo_t *)canvas->cr;
    cairo_surface_flush(win->cairo_surf);
    cairo_destroy(cr);
    cairo_surface_destroy(win->cairo_surf);
    win->cairo_surf = NULL;

    ShmBuf *b = &win->bufs[win->active_buf];
    b->busy = true;

    if (win->viewport) {
        wp_viewport_set_destination(win->viewport, win->width, win->height);
    } else {
        int32_t iscale = (int32_t)((win->scale_120 + 60) / 120);
        if (iscale < 1) iscale = 1;
        wl_surface_set_buffer_scale(win->surface, iscale);
    }

    wl_surface_attach(win->surface, b->buffer, 0, 0);
    wl_surface_damage_buffer(win->surface, 0, 0, b->width, b->height);
    wl_surface_commit(win->surface);
    wl_display_flush(win->app->display);
    win->active_buf = -1;
}

// ---- Cairo / Pango Drawing Primitives ----
static inline void set_rgb(cairo_t *cr, uint32_t rgb, double alpha) {
    double r = ((rgb >> 16) & 0xFF) / 255.0;
    double g = ((rgb >> 8) & 0xFF) / 255.0;
    double b = (rgb & 0xFF) / 255.0;
    cairo_set_source_rgba(cr, r, g, b, alpha);
}

void kdx_draw_rect(void *cr_p, double x, double y, double w, double h, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    cairo_rectangle(cr, x, y, w, h);
    cairo_fill(cr);
}

static void round_rect_path(cairo_t *cr, double x, double y, double w, double h, double r) {
    if (r > w / 2.0) r = w / 2.0;
    if (r > h / 2.0) r = h / 2.0;
    cairo_new_sub_path(cr);
    cairo_arc(cr, x + w - r, y + r, r, -M_PI / 2.0, 0);
    cairo_arc(cr, x + w - r, y + h - r, r, 0, M_PI / 2.0);
    cairo_arc(cr, x + r, y + h - r, r, M_PI / 2.0, M_PI);
    cairo_arc(cr, x + r, y + r, r, M_PI, 3.0 * M_PI / 2.0);
    cairo_close_path(cr);
}

void kdx_draw_round_rect(void *cr_p, double x, double y, double w, double h, double r, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    round_rect_path(cr, x, y, w, h, r);
    cairo_fill(cr);
}

void kdx_stroke_round_rect(void *cr_p, double x, double y, double w, double h, double r, double lw, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    cairo_set_line_width(cr, lw);
    round_rect_path(cr, x, y, w, h, r);
    cairo_stroke(cr);
}

void kdx_clip_round_rect(void *cr_p, double x, double y, double w, double h, double r) {
    cairo_t *cr = (cairo_t *)cr_p;
    round_rect_path(cr, x, y, w, h, r);
    cairo_clip(cr);
}

void kdx_stroke_circle(void *cr_p, double cx, double cy, double r, double lw, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    cairo_set_line_width(cr, lw);
    cairo_new_sub_path(cr);
    cairo_arc(cr, cx, cy, r, 0, 2.0 * M_PI);
    cairo_stroke(cr);
}

// Icon paths: build with move/line/curve/close, then stroke (round caps and joins).
void kdx_path_begin(void *cr_p) { cairo_new_path((cairo_t *)cr_p); }
void kdx_path_move(void *cr_p, double x, double y) { cairo_move_to((cairo_t *)cr_p, x, y); }
void kdx_path_line(void *cr_p, double x, double y) { cairo_line_to((cairo_t *)cr_p, x, y); }
void kdx_path_curve(void *cr_p, double x1, double y1, double x2, double y2, double x3, double y3) {
    cairo_curve_to((cairo_t *)cr_p, x1, y1, x2, y2, x3, y3);
}
void kdx_path_close(void *cr_p) { cairo_close_path((cairo_t *)cr_p); }
void kdx_path_stroke(void *cr_p, double lw, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    cairo_set_line_width(cr, lw);
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_ROUND);
    cairo_set_line_join(cr, CAIRO_LINE_JOIN_ROUND);
    cairo_stroke(cr);
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_BUTT);
    cairo_set_line_join(cr, CAIRO_LINE_JOIN_MITER);
}

// Makes a private font file visible to Pango without installing it system-wide.
// Returns false (and changes nothing) if fontconfig rejects the file.
bool kdx_font_register(const char *path) {
    if (!path) return false;
    if (!FcConfigAppFontAddFile(NULL, (const FcChar8 *)path)) return false;
    PangoFontMap *map = pango_cairo_font_map_get_default();
    if (map && PANGO_IS_FC_FONT_MAP(map)) pango_fc_font_map_config_changed(PANGO_FC_FONT_MAP(map));
    return true;
}

void kdx_draw_circle(void *cr_p, double cx, double cy, double r, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    cairo_arc(cr, cx, cy, r, 0, 2.0 * M_PI);
    cairo_fill(cr);
}

void kdx_draw_line(void *cr_p, double x1, double y1, double x2, double y2, double lw, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    cairo_set_line_width(cr, lw);
    cairo_move_to(cr, x1, y1);
    cairo_line_to(cr, x2, y2);
    cairo_stroke(cr);
}

void kdx_clip_rect(void *cr_p, double x, double y, double w, double h) {
    cairo_t *cr = (cairo_t *)cr_p;
    cairo_rectangle(cr, x, y, w, h);
    cairo_clip(cr);
}

void kdx_save(void *cr_p) { cairo_save((cairo_t *)cr_p); }
void kdx_restore(void *cr_p) { cairo_restore((cairo_t *)cr_p); }
void kdx_translate(void *cr_p, double tx, double ty) { cairo_translate((cairo_t *)cr_p, tx, ty); }
void kdx_scale(void *cr_p, double sx, double sy) { cairo_scale((cairo_t *)cr_p, sx, sy); }

KdxLayout *kdx_layout_new(void *cr_p, const char *font_family, double font_size_pt,
                          const char *text, size_t text_len, int32_t wrap_width_px) {
    cairo_t *cr = (cairo_t *)cr_p;
    PangoLayout *pl = pango_cairo_create_layout(cr);
    PangoFontDescription *desc = pango_font_description_new();
    pango_font_description_set_family(desc, font_family ? font_family : "monospace");
    pango_font_description_set_size(desc, (int)(font_size_pt * PANGO_SCALE));
    pango_layout_set_font_description(pl, desc);
    pango_font_description_free(desc);

    if (wrap_width_px > 0) {
        pango_layout_set_width(pl, wrap_width_px * PANGO_SCALE);
        pango_layout_set_wrap(pl, PANGO_WRAP_WORD_CHAR);
    } else {
        pango_layout_set_width(pl, -1);
    }

    pango_layout_set_text(pl, text ? text : "", (int)text_len);

    KdxLayout *kl = (KdxLayout *)malloc(sizeof(KdxLayout));
    kl->layout = pl;
    return kl;
}

// UI chrome text: fractional glyph advances (no integer rounding) so small labels keep even spacing.
void kdx_layout_set_subpixel(KdxLayout *kl) {
    if (kl && kl->layout) {
        pango_context_set_round_glyph_positions(pango_layout_get_context(kl->layout), FALSE);
    }
}

void kdx_layout_free(KdxLayout *kl) {
    if (!kl) return;
    if (kl->layout) g_object_unref(kl->layout);
    free(kl);
}

void kdx_layout_get_size(KdxLayout *kl, double *out_w, double *out_h) {
    int w = 0, h = 0;
    pango_layout_get_pixel_size(kl->layout, &w, &h);
    if (out_w) *out_w = (double)w;
    if (out_h) *out_h = (double)h;
}

void kdx_layout_draw(void *cr_p, KdxLayout *kl, double x, double y, uint32_t rgb, double alpha) {
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);
    cairo_move_to(cr, x, y);
    pango_cairo_show_layout(cr, kl->layout);
}

void kdx_layout_index_to_pos(KdxLayout *kl, int byte_idx, double *out_x, double *out_y, double *out_h) {
    PangoRectangle pos;
    pango_layout_index_to_pos(kl->layout, byte_idx, &pos);
    if (out_x) *out_x = (double)pos.x / PANGO_SCALE;
    if (out_y) *out_y = (double)pos.y / PANGO_SCALE;
    if (out_h) *out_h = (double)pos.height / PANGO_SCALE;
}

int kdx_layout_xy_to_index(KdxLayout *kl, double x, double y) {
    int idx = 0, trailing = 0;
    pango_layout_xy_to_index(kl->layout, (int)(x * PANGO_SCALE), (int)(y * PANGO_SCALE), &idx, &trailing);
    const char *text = pango_layout_get_text(kl->layout);
    int len = (int)strlen(text);
    while (trailing > 0 && idx < len) {
        idx++;
        while (idx < len && (((unsigned char)text[idx]) & 0xC0) == 0x80) {
            idx++;
        }
        trailing--;
    }
    return idx;
}

void kdx_layout_draw_selection(void *cr_p, KdxLayout *kl, double ox, double oy,
                               int start_byte, int end_byte, uint32_t rgb, double alpha) {
    if (start_byte >= end_byte) return;
    cairo_t *cr = (cairo_t *)cr_p;
    set_rgb(cr, rgb, alpha);

    PangoLayoutIter *iter = pango_layout_get_iter(kl->layout);
    do {
        PangoLayoutLine *line = pango_layout_iter_get_line_readonly(iter);
        int ls = line->start_index;
        int le = ls + line->length;
        if (end_byte <= ls) break;
        if (start_byte > le) continue;

        int s = start_byte > ls ? start_byte : ls;
        int e = end_byte < le ? end_byte : le;
        int x0 = 0, x1 = 0;
        pango_layout_line_index_to_x(line, s, FALSE, &x0);
        pango_layout_line_index_to_x(line, e, FALSE, &x1);
        if (x1 < x0) { int t = x0; x0 = x1; x1 = t; }

        PangoRectangle rect;
        pango_layout_iter_get_line_extents(iter, NULL, &rect);
        double rx = ox + (double)x0 / PANGO_SCALE;
        double ry = oy + (double)rect.y / PANGO_SCALE;
        double rw = (double)(x1 - x0) / PANGO_SCALE;
        double rh = (double)rect.height / PANGO_SCALE;
        if (end_byte > le) rw += 6.0; // indicate newline selected
        if (rw > 0.5) {
            cairo_rectangle(cr, rx, ry, rw, rh);
            cairo_fill(cr);
        }
    } while (pango_layout_iter_next_line(iter));
    pango_layout_iter_free(iter);
}

int kdx_hit_test(const char *font_family, double font_size_pt, const char *text,
                 size_t text_len, int32_t wrap_width_px, double x, double y) {
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 1, 1);
    cairo_t *cr = cairo_create(s);
    KdxLayout *kl = kdx_layout_new(cr, font_family, font_size_pt, text, text_len, wrap_width_px);
    int idx = kdx_layout_xy_to_index(kl, x, y);
    kdx_layout_free(kl);
    cairo_destroy(cr);
    cairo_surface_destroy(s);
    return idx;
}

double kdx_measure_line_height(const char *font_family, double font_size_pt) {
    cairo_surface_t *s = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, 1, 1);
    cairo_t *cr = cairo_create(s);
    KdxLayout *kl = kdx_layout_new(cr, font_family, font_size_pt, "Ag", 2, -1);
    double w = 0, h = 18.0;
    kdx_layout_get_size(kl, &w, &h);
    kdx_layout_free(kl);
    cairo_destroy(cr);
    cairo_surface_destroy(s);
    return h > 6.0 ? h : 18.0;
}

