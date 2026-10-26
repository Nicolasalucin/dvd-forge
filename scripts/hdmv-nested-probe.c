/*
 * Minimal libbluray probe for the nested HDMV IG fixture.
 *
 * Verifies root -> episode menu -> back to root, chapter-2 start position and
 * episode-2 start position. Build:
 *   cc -O2 -o /tmp/hdmv-nested-probe scripts/hdmv-nested-probe.c $(pkg-config --cflags --libs libbluray)
 */
#include <stdio.h>
#include <stdint.h>
#include <unistd.h>
#include <string.h>
#include <libbluray/bluray.h>
#include <libbluray/overlay.h>
#include <libbluray/keys.h>

static int draws = 0;
static void cb(void *h, const BD_OVERLAY *ev) {
  (void)h;
  if (!ev || ev->plane != BD_OVERLAY_IG) return;
  if (ev->cmd == BD_OVERLAY_DRAW) draws++;
}

static int pump(BLURAY *bd, uint8_t *buf, int *pl, int *menu, int reads) {
  for (int i = 0; i < reads; i++) {
    BD_EVENT ev = {0, 0};
    bd_read_ext(bd, buf, 6144, &ev);
    if (ev.event == BD_EVENT_PLAYLIST) *pl = ev.param;
    if (ev.event == BD_EVENT_MENU) *menu = ev.param;
    if (ev.event == BD_EVENT_TITLE) { /* printf("TITLE %u\n", ev.param); */ }
    usleep(1000);
  }
  return 0;
}

static int wait_menu(BLURAY *bd, uint8_t *buf, int *pl, int *menu, int wanted_pl) {
  for (int i = 0; i < 4000; i++) {
    pump(bd, buf, pl, menu, 1);
    if (*pl == wanted_pl && *menu == 1 && draws >= 2) return 0;
  }
  return -1;
}

static uint64_t wait_time(BLURAY *bd, uint8_t *buf, int *pl, int *menu, uint64_t target) {
  uint64_t t = 0;
  for (int i = 0; i < 12000; i++) {
    pump(bd, buf, pl, menu, 1);
    t = bd_tell_time(bd);
    if (t >= target) return t;
    if (*menu == 1 && *pl != 0) break;
  }
  return t;
}

static uint64_t pts = 54000000;
static void press_key(BLURAY *bd, uint8_t *buf, int *pl, int *menu, int key) {
  pts += 90000;
  bd_user_input(bd, pts, key);
  pump(bd, buf, pl, menu, 12);
  usleep(150000);
}

int main(int argc, char **argv) {
  if (argc < 2) return 2;
  BLURAY *bd = bd_open(argv[1], NULL);
  if (!bd) { printf("FAIL open\n"); return 2; }
  bd_register_overlay_proc(bd, NULL, cb);
  if (!bd_play(bd)) { printf("FAIL play\n"); return 2; }
  uint8_t buf[6144];
  int pl = -1, menu = 0;
  if (wait_menu(bd, buf, &pl, &menu, 1) != 0) { printf("FAIL root pl=%d menu=%d draws=%d\n", pl, menu, draws); return 3; }
  printf("root pl=%d draws=%d\n", pl, draws);

  int before = draws;
  press_key(bd, buf, &pl, &menu, BD_VK_ENTER);
  if (wait_menu(bd, buf, &pl, &menu, 2) != 0) { printf("FAIL submenu pl=%d menu=%d\n", pl, menu); return 4; }
  printf("submenu pl=%d draws_delta=%d\n", pl, draws - before);

  // Return button is the fourth button: right x3, ENTER.
  press_key(bd, buf, &pl, &menu, BD_VK_RIGHT); press_key(bd, buf, &pl, &menu, BD_VK_RIGHT); press_key(bd, buf, &pl, &menu, BD_VK_RIGHT);
  press_key(bd, buf, &pl, &menu, BD_VK_ENTER);
  if (wait_menu(bd, buf, &pl, &menu, 1) != 0) { printf("FAIL return pl=%d menu=%d\n", pl, menu); return 5; }
  printf("return pl=%d\n", pl);

  // Chapter button is the third button: right x2, ENTER.
  press_key(bd, buf, &pl, &menu, BD_VK_ENTER);
  if (wait_menu(bd, buf, &pl, &menu, 2) != 0) { printf("FAIL submenu2\n"); return 6; }
  press_key(bd, buf, &pl, &menu, BD_VK_RIGHT); press_key(bd, buf, &pl, &menu, BD_VK_RIGHT); press_key(bd, buf, &pl, &menu, BD_VK_ENTER);
  // Wait until the movie playlist starts.
  for (int i = 0; i < 2000; i++) { pump(bd, buf, &pl, &menu, 1); if (pl == 0) break; }
  uint64_t t = wait_time(bd, buf, &pl, &menu, 9ULL * 90000ULL);
  printf("chapter2 t=%llu (%.2fs) pl=%d\n", (unsigned long long)t, t / 90000.0, pl);
  if (t < 8ULL * 90000ULL) { printf("FAIL chapter start\n"); return 7; }
  press_key(bd, buf, &pl, &menu, BD_VK_ROOT_MENU);
  if (wait_menu(bd, buf, &pl, &menu, 1) != 0) { printf("FAIL root after chapter\n"); return 8; }

  // Episode 2 button is the second button: right x1, ENTER.
  press_key(bd, buf, &pl, &menu, BD_VK_ENTER);
  if (wait_menu(bd, buf, &pl, &menu, 2) != 0) { printf("FAIL submenu3\n"); return 9; }
  press_key(bd, buf, &pl, &menu, BD_VK_RIGHT); press_key(bd, buf, &pl, &menu, BD_VK_ENTER);
  for (int i = 0; i < 2000; i++) { pump(bd, buf, &pl, &menu, 1); if (pl == 0) break; }
  t = wait_time(bd, buf, &pl, &menu, 19ULL * 90000ULL);
  printf("episode2 t=%llu (%.2fs) pl=%d\n", (unsigned long long)t, t / 90000.0, pl);
  if (t < 18ULL * 90000ULL) { printf("FAIL episode start\n"); return 10; }

  press_key(bd, buf, &pl, &menu, BD_VK_ROOT_MENU);
  if (wait_menu(bd, buf, &pl, &menu, 1) != 0) { printf("FAIL final root\n"); return 11; }
  printf("RESULT ok pl=%d draws=%d\n", pl, draws);
  bd_close(bd);
  return 0;
}
