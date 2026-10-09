/* SPDX-License-Identifier: GPL-2.0-or-later (built against libdvdnav's internals) */
/*
 * libdvdnav oracle: plays a disc headlessly with scripted input and prints an
 * event trace in the same format as `menuseum trace` (crates/cli/src/trace.rs),
 * so the two can be diffed.
 *
 * usage: oracle <disc> [-i 'down activate ...' | --inputs FILE] [--max-events N] [--seed N]
 */
#include <inttypes.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>

#include "dvdnav/dvdnav.h"
#include <dvdread/dvd_reader.h>
#include <dvdread/nav_types.h>
#include "vm/decoder.h"
#include "vm/vm.h"
#include "vm/getset.h"
#include "dvdnav_internal.h"

/* ---- deterministic replacements for rand() and gettimeofday() ---- */

static uint64_t rng_state;
static uint64_t now_ms;

/* SplitMix64 truncated to 31 bits; identical to menuseum_core::vm::Rng. */
int oracle_rand(void) {
  uint64_t z;
  rng_state += 0x9e3779b97f4a7c15ULL;
  z = rng_state;
  z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
  z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
  return (int)((z ^ (z >> 31)) >> 33);
}

int oracle_gettimeofday(struct timeval *tv, void *tz) {
  (void)tz;
  tv->tv_sec = now_ms / 1000;
  tv->tv_usec = (now_ms % 1000) * 1000;
  return 0;
}

/* ---- input script ---- */

enum kind { UP, DOWN, LEFT, RIGHT, ACTIVATE, SELECT, MENU };
struct input { enum kind kind; int arg; char name[32]; };

static struct input *inputs;
static int n_inputs, next_input;

static void parse_script(const char *text) {
  char *copy = strdup(text), *line, *save_line;
  for (line = strtok_r(copy, "\n", &save_line); line; line = strtok_r(NULL, "\n", &save_line)) {
    char *hash = strchr(line, '#'), *tok, *save_tok;
    if (hash) *hash = 0;
    for (tok = strtok_r(line, " \t\r", &save_tok); tok; tok = strtok_r(NULL, " \t\r", &save_tok)) {
      struct input in;
      char *colon = strchr(tok, ':');
      const char *arg = colon ? colon + 1 : "";
      size_t wl = colon ? (size_t)(colon - tok) : strlen(tok);
      memset(&in, 0, sizeof in);
#define IS(w) (wl == strlen(w) && !strncmp(tok, w, wl))
      if (IS("up")) { in.kind = UP; strcpy(in.name, "up"); }
      else if (IS("down")) { in.kind = DOWN; strcpy(in.name, "down"); }
      else if (IS("left")) { in.kind = LEFT; strcpy(in.name, "left"); }
      else if (IS("right")) { in.kind = RIGHT; strcpy(in.name, "right"); }
      else if (IS("activate") || IS("ok") || IS("enter")) { in.kind = ACTIVATE; strcpy(in.name, "activate"); }
      else if (IS("select")) { in.kind = SELECT; in.arg = atoi(arg); snprintf(in.name, sizeof in.name, "select:%d", in.arg); }
      else if (IS("menu")) {
        in.kind = MENU;
        if (!strcmp(arg, "escape")) in.arg = DVD_MENU_Escape;
        else if (!strcmp(arg, "title")) in.arg = DVD_MENU_Title;
        else if (!strcmp(arg, "root")) in.arg = DVD_MENU_Root;
        else if (!strcmp(arg, "subpicture")) in.arg = DVD_MENU_Subpicture;
        else if (!strcmp(arg, "audio")) in.arg = DVD_MENU_Audio;
        else if (!strcmp(arg, "angle")) in.arg = DVD_MENU_Angle;
        else if (!strcmp(arg, "part")) in.arg = DVD_MENU_Part;
        else { fprintf(stderr, "unknown menu in %s\n", tok); exit(2); }
        snprintf(in.name, sizeof in.name, "menu:%s", arg);
      } else { fprintf(stderr, "unknown input %s\n", tok); exit(2); }
#undef IS
      inputs = realloc(inputs, sizeof *inputs * (n_inputs + 1));
      inputs[n_inputs++] = in;
    }
  }
  free(copy);
}

static int apply(dvdnav_t *nav, const struct input *in) {
  pci_t *pci = dvdnav_get_current_nav_pci(nav);
  switch (in->kind) {
  case UP: return dvdnav_upper_button_select(nav, pci) == DVDNAV_STATUS_OK;
  case DOWN: return dvdnav_lower_button_select(nav, pci) == DVDNAV_STATUS_OK;
  case LEFT: return dvdnav_left_button_select(nav, pci) == DVDNAV_STATUS_OK;
  case RIGHT: return dvdnav_right_button_select(nav, pci) == DVDNAV_STATUS_OK;
  case ACTIVATE: return dvdnav_button_activate(nav, pci) == DVDNAV_STATUS_OK;
  case SELECT: return dvdnav_button_select(nav, pci, in->arg) == DVDNAV_STATUS_OK;
  case MENU: return dvdnav_menu_call(nav, (DVDMenuID_t)in->arg) == DVDNAV_STATUS_OK;
  }
  return 0;
}

/* ---- trace output ---- */

static void state(dvdnav_t *nav, int full) {
  vm_t *vm = nav->vm;
  dvd_state_t *s = &vm->state;
  int i, pgcn = 0;
  if (s->pgc && s->domain != DVD_DOMAIN_FirstPlay) pgcn = get_PGCN(vm);
  else if (s->pgc && !vm->vmgi->first_play_pgc) pgcn = get_PGCN(vm);
  printf("d=%d vts=%d pgc=%d pg=%d cell=%d btn=%d", s->domain, s->vtsN < 0 ? 0 : s->vtsN, pgcn, s->pgN,
         s->cellN, s->registers.SPRM[8] >> 10);
  if (full) {
    printf(" S=");
    for (i = 0; i < 24; i++) printf("%s%04x", i ? "," : "", s->registers.SPRM[i]);
    printf(" G=");
    for (i = 0; i < 16; i++) printf("%s%04x", i ? "," : "", s->registers.GPRM[i]);
  }
  printf("\n");
}

static int input_point(dvdnav_t *nav) {
  if (next_input >= n_inputs) {
    printf("END waiting-for-input\n");
    return 0;
  }
  {
    const struct input *in = &inputs[next_input++];
    int ok = apply(nav, in);
    printf("INPUT %s %s\n", in->name, ok ? "ok" : "ignored");
  }
  return 1;
}

static void quiet_log(void *priv, dvdnav_logger_level_t level, const char *fmt, va_list ap) {
  (void)priv; (void)level; (void)fmt; (void)ap;
}

static char *slurp(const char *path) {
  FILE *f = fopen(path, "rb");
  long n;
  char *buf;
  if (!f) { perror(path); exit(2); }
  fseek(f, 0, SEEK_END);
  n = ftell(f);
  fseek(f, 0, SEEK_SET);
  buf = malloc(n + 1);
  if (fread(buf, 1, n, f) != (size_t)n) { perror(path); exit(2); }
  buf[n] = 0;
  fclose(f);
  return buf;
}

int main(int argc, char **argv) {
  dvdnav_t *nav;
  dvdnav_logger_cb log = { quiet_log };
  uint8_t buf[DVD_VIDEO_LB_LEN];
  long max_events = 200000, events = 0;
  int i;

  if (argc < 2) {
    fprintf(stderr, "usage: oracle <disc> [-i 'inputs' | --inputs FILE] [--max-events N] [--seed N]\n");
    return 2;
  }
  for (i = 2; i < argc; i++) {
    if (i + 1 >= argc) { fprintf(stderr, "%s needs a value\n", argv[i]); return 2; }
    if (!strcmp(argv[i], "-i")) parse_script(argv[++i]);
    else if (!strcmp(argv[i], "--inputs")) parse_script(slurp(argv[++i]));
    else if (!strcmp(argv[i], "--max-events")) max_events = atol(argv[++i]);
    else if (!strcmp(argv[i], "--seed")) rng_state = strtoull(argv[++i], NULL, 10);
    else { fprintf(stderr, "unknown option %s\n", argv[i]); return 2; }
  }

  if (dvdnav_open2(&nav, NULL, getenv("ORACLE_VERBOSE") ? NULL : &log, argv[1]) != DVDNAV_STATUS_OK) {
    fprintf(stderr, "cannot open %s\n", argv[1]);
    return 1;
  }
  dvdnav_set_readahead_flag(nav, 0);

  for (;;) {
    int32_t event, len;
    if (events >= max_events) { printf("END max-events\n"); break; }
    events++;
    if (dvdnav_get_next_block(nav, buf, &event, &len) != DVDNAV_STATUS_OK) {
      printf("ERROR %s\n", dvdnav_err_to_string(nav));
      break;
    }
    switch (event) {
    case DVDNAV_BLOCK_OK:
    case DVDNAV_NOP:
      events--; /* blocks are not events in the Rust engine */
      if (event == DVDNAV_NOP) events++;
      continue;
    case DVDNAV_STOP:
      printf("STOP "); state(nav, 1);
      goto done;
    case DVDNAV_WAIT:
      dvdnav_wait_skip(nav);
      continue;
    case DVDNAV_HOP_CHANNEL:
      printf("HOP "); state(nav, 1);
      break;
    case DVDNAV_HIGHLIGHT: {
      dvdnav_highlight_event_t *h = (dvdnav_highlight_event_t *)buf;
      printf("HIGHLIGHT %d ", h->buttonN); state(nav, 1);
      break;
    }
    case DVDNAV_VTS_CHANGE: {
      dvdnav_vts_change_event_t *v = (dvdnav_vts_change_event_t *)buf;
      printf("VTS %d:%d->%d:%d ", v->old_domain, v->old_vtsN < 0 ? 0 : v->old_vtsN, v->new_domain,
             v->new_vtsN < 0 ? 0 : v->new_vtsN);
      state(nav, 1);
      break;
    }
    case DVDNAV_CELL_CHANGE: {
      dvdnav_cell_change_event_t *c = (dvdnav_cell_change_event_t *)buf;
      printf("CELL %d pg=%d ", c->cellN, c->pgN); state(nav, 1);
      break;
    }
    case DVDNAV_SPU_CLUT_CHANGE: printf("CLUT "); state(nav, 0); break;
    case DVDNAV_SPU_STREAM_CHANGE: printf("SPU "); state(nav, 0); break;
    case DVDNAV_AUDIO_STREAM_CHANGE: printf("AUDIO "); state(nav, 0); break;
    case DVDNAV_STILL_FRAME: {
      dvdnav_still_event_t *st = (dvdnav_still_event_t *)buf;
      int secs = st->length > 0xff ? 0xff : st->length;
      printf("STILL %d ", secs); state(nav, 1);
      if (secs != 0xff) {
        now_ms += (uint64_t)secs * 1000;
        dvdnav_still_skip(nav);
      } else if (!input_point(nav)) {
        goto done;
      }
      break;
    }
    case DVDNAV_NAV_PACKET: {
      pci_t *pci = dvdnav_get_current_nav_pci(nav);
      printf("NAV %u hli=%d btns=%d ", pci->pci_gi.nv_pck_lbn, pci->hli.hl_gi.hli_ss, pci->hli.hl_gi.btn_ns);
      state(nav, 0);
      now_ms += (uint32_t)(pci->pci_gi.vobu_e_ptm - pci->pci_gi.vobu_s_ptm) / 90;
      if (pci->hli.hl_gi.hli_ss && pci->hli.hl_gi.btn_ns && !input_point(nav)) goto done;
      break;
    }
    default:
      printf("UNKNOWN-EVENT %d\n", event);
      break;
    }
  }
done:
  dvdnav_close(nav);
  return 0;
}
