"""Drives scripts/shortcuts.sh: one real key press per default shortcut."""
import json, os, subprocess, sys, time

cli, socket, poster_source, engine_pid = sys.argv[1:5]
poster = subprocess.Popen(["swift", poster_source], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
CODES = {"h": 4, "j": 38, "k": 40, "l": 37, "left": 123, "right": 124, "up": 126, "down": 125,
         "r": 15, "f": 3, "c": 8, "equal": 24, "minus": 27, "0": 29, "]": 30, "[": 33,
         "t": 17, "n": 45, "p": 35, "q": 12,
         "1": 18, "2": 19, "3": 20, "4": 21, "5": 23, "6": 22, "7": 26, "8": 28, "9": 25}
failures = []
timings = []


def query():
    out = subprocess.run([cli, "--socket", socket, "query", "state", "--json"], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


def strip():
    body = query()
    for display in body["displays"]:
        for item in display["strips"]:
            if item["columns"] or item["pinned_left"] or item["pinned_right"]:
                return body, display, item
    empty = {"columns": [], "pinned_left": None, "pinned_right": None, "focused_column": 0, "focused_window": None}
    display = body["displays"][0] if body["displays"] else {}
    return body, display, (display.get("strips") or [empty])[0]


def run(*args):
    subprocess.run([cli, "--socket", socket, *args], capture_output=True, check=False)


def column_of(item, wid):
    for index, column in enumerate(item["columns"]):
        if any(window["id"] == wid for window in column["windows"]):
            return index
    return None


def focused(item):
    return item["focused_column"]


def press(label, key, mods, check, timeout=1.5):
    start = time.perf_counter()
    poster.stdin.write(f"{CODES[key]} {mods}\n")
    poster.stdin.flush()
    poster.stdout.readline()
    deadline = start + timeout
    last = None
    while time.perf_counter() < deadline:
        try:
            body, display, item = strip()
            ok, last = check(body, display, item)
            if ok:
                ms = (time.perf_counter() - start) * 1000
                timings.append(ms)
                print(f"ok    {label:<34} {ms:5.0f} ms")
                time.sleep(0.2)
                return True
        except Exception as err:  # noqa: BLE001
            last = repr(err)
        time.sleep(0.01)
    failures.append(label)
    print(f"FAIL  {label:<34} last={last}")
    time.sleep(0.2)
    return False


def settle():
    time.sleep(0.4)


for _ in range(80):
    try:
        _, _, item = strip()
        if sum(len(c["windows"]) for c in item["columns"]) >= 5:
            break
    except Exception:  # noqa: BLE001
        pass
    time.sleep(0.25)
_, _, item = strip()
count = len(item["columns"])
print(f"managed columns={count}")
if count < 5:
    sys.exit("expected 5 columns")
last = count - 1

run("focus_index", "1")
settle()
want = lambda n: (lambda b, d, s: (focused(s) == n, focused(s)))
press("⌃⌥→ focus right", "right", "ctrl,alt", want(1))
press("⌃⌥L focus right", "l", "ctrl,alt", want(2))
press("⌃⌥← focus left", "left", "ctrl,alt", want(1))
press("⌃⌥H focus left", "h", "ctrl,alt", want(0))
press("⌃⌥⌘→ focus last", "right", "ctrl,alt,cmd", want(last))
press("⌃⌥⌘← focus first", "left", "ctrl,alt,cmd", want(0))
for n in range(1, 10):
    target = last if n == 9 else min(n - 1, last)
    if n > 1 and target == min(n - 2, last) and n != 9:
        run("focus_index", "1")
        settle()
    press(f"⌃⌥{n} focus column {n}", str(n), "ctrl,alt", want(target))

run("focus_index", "2")
settle()
_, _, item = strip()
wid = item["columns"][1]["windows"][0]["id"]
at = lambda n: (lambda b, d, s: (column_of(s, wid) == n, column_of(s, wid)))
press("⌃⌥⇧→ move right", "right", "ctrl,alt,shift", at(2))
press("⌃⌥⇧L move right", "l", "ctrl,alt,shift", at(3))
press("⌃⌥⇧← move left", "left", "ctrl,alt,shift", at(2))
press("⌃⌥⇧H move left", "h", "ctrl,alt,shift", at(1))
press("⌃⌥⇧⌘→ move to last", "right", "ctrl,alt,shift,cmd", at(last))
press("⌃⌥⇧⌘← move to first", "left", "ctrl,alt,shift,cmd", at(0))
press("⌃⌥⇧3 move to column 3", "3", "ctrl,alt,shift", at(2))
press("⌃⌥⇧9 move to last", "9", "ctrl,alt,shift", at(last))

run("focus_index", "3")
settle()
ratio = lambda s: s["columns"][focused(s)]["width_ratio"]
_, _, item = strip()
base = ratio(item)
press("⌃⌥R cycle width", "r", "ctrl,alt", lambda b, d, s: (abs(ratio(s) - base) > 0.01, ratio(s)))
press("⌃⌥F full width", "f", "ctrl,alt", lambda b, d, s: (abs(ratio(s) - 1.0) < 0.01, ratio(s)))
press("⌃⌥F full width again", "f", "ctrl,alt", lambda b, d, s: (ratio(s) < 0.99, ratio(s)))
_, _, item = strip()
base = ratio(item)
press("⌃⌥= wider", "equal", "ctrl,alt", lambda b, d, s: (ratio(s) > base + 0.01, ratio(s)))
_, _, item = strip()
base = ratio(item)
press("⌃⌥- narrower", "minus", "ctrl,alt", lambda b, d, s: (ratio(s) < base - 0.01, ratio(s)))


def centered(body, display, item):
    vx, _, vw, _ = display["visible_frame"]
    frame = item["columns"][focused(item)]["windows"][0]["frame"]
    middle = frame[0] + frame[2] / 2
    return abs(middle - (vx + vw / 2)) < 3, round(middle - (vx + vw / 2), 1)


press("⌃⌥C center", "c", "ctrl,alt", centered)

run("focus_index", "2")
settle()
_, _, item = strip()
moving = item["columns"][1]["windows"][0]["id"]
stacked = lambda s: max(len(c["windows"]) for c in s["columns"])
press("⌃⌥] stack into left column", "]", "ctrl,alt", lambda b, d, s: (len(s["columns"]) == count - 1 and column_of(s, moving) == 0, [len(c["windows"]) for c in s["columns"]]))
_, _, item = strip()
top = item["focused_window"]
press("⌃⌥↑/K focus up in stack", "k", "ctrl,alt", lambda b, d, s: (s["focused_window"] != top, s["focused_window"]))
press("⌃⌥↓/J focus down in stack", "j", "ctrl,alt", lambda b, d, s: (s["focused_window"] == top, s["focused_window"]))
press("⌃⌥↑ focus up in stack", "up", "ctrl,alt", lambda b, d, s: (s["focused_window"] != top, s["focused_window"]))
press("⌃⌥↓ focus down in stack", "down", "ctrl,alt", lambda b, d, s: (s["focused_window"] == top, s["focused_window"]))
height = lambda s: next(w["height_ratio"] for c in s["columns"] for w in c["windows"] if w["id"] == s["focused_window"])
_, _, item = strip()
base = height(item)
press("⌃⌥⇧= taller", "equal", "ctrl,alt,shift", lambda b, d, s: (height(s) > base + 0.01, height(s)))
_, _, item = strip()
base = height(item)
press("⌃⌥⇧- shorter", "minus", "ctrl,alt,shift", lambda b, d, s: (height(s) < base - 0.01, height(s)))
press("⌃⌥⇧- shorter again", "minus", "ctrl,alt,shift", lambda b, d, s: (height(s) < base - 0.02, height(s)))
heights = lambda s: [w["height_ratio"] for w in s["columns"][focused(s)]["windows"]]
press("⌃⌥0 equalize heights", "0", "ctrl,alt", lambda b, d, s: (max(heights(s)) - min(heights(s)) < 0.01, heights(s)))
order = lambda s: column_of(s, s["focused_window"])
_, _, item = strip()
bottom = item["focused_window"]
press("⌃⌥⇧K move up in stack", "k", "ctrl,alt,shift", lambda b, d, s: ([w["id"] for w in s["columns"][focused(s)]["windows"]][0] == bottom, [w["id"] for w in s["columns"][focused(s)]["windows"]]))
press("⌃⌥⇧J move down in stack", "j", "ctrl,alt,shift", lambda b, d, s: ([w["id"] for w in s["columns"][focused(s)]["windows"]][-1] == bottom, [w["id"] for w in s["columns"][focused(s)]["windows"]]))
mode = lambda s: s["columns"][focused(s)]["mode"]
press("⌃⌥⇧T tabbed", "t", "ctrl,alt,shift", lambda b, d, s: (mode(s) == "tabbed", mode(s)))
press("⌃⌥⇧T stacked", "t", "ctrl,alt,shift", lambda b, d, s: (mode(s) == "stacked", mode(s)))
press("⌃⌥⇧] pop out right", "]", "ctrl,alt,shift", lambda b, d, s: (len(s["columns"]) == count and stacked(s) == 1, len(s["columns"])))
run("focus_index", "2")
settle()
press("⌃⌥[ stack into right column", "[", "ctrl,alt", lambda b, d, s: (len(s["columns"]) == count - 1, len(s["columns"])))
press("⌃⌥⇧[ pop out left", "[", "ctrl,alt,shift", lambda b, d, s: (len(s["columns"]) == count, len(s["columns"])))

run("focus_index", "2")
settle()
press("⌃⌥P pin left", "p", "ctrl,alt", lambda b, d, s: (s["pinned_left"] is not None, s["pinned_left"] is not None))
press("⌃⌥P unpin left", "p", "ctrl,alt", lambda b, d, s: (s["pinned_left"] is None and len(s["columns"]) == count, len(s["columns"])))
press("⌃⌥⇧P pin right", "p", "ctrl,alt,shift", lambda b, d, s: (s["pinned_right"] is not None, s["pinned_right"] is not None))
press("⌃⌥⇧P unpin right", "p", "ctrl,alt,shift", lambda b, d, s: (s["pinned_right"] is None and len(s["columns"]) == count, len(s["columns"])))

run("focus_index", "2")
settle()
press("⌃⌥T float", "t", "ctrl,alt", lambda b, d, s: (len(b["floating"]) == 1 and len(s["columns"]) == count - 1, len(b["floating"])))
time.sleep(0.5)
press("⌃⌥T back into the strip", "t", "ctrl,alt", lambda b, d, s: (len(b["floating"]) == 0 and len(s["columns"]) == count, len(b["floating"])))

press("⌃⌥N next display (one display: no-op)", "n", "ctrl,alt", lambda b, d, s: (len(s["columns"]) == count, len(s["columns"])))
press("⌃⌥Q stop managing", "q", "ctrl,alt", lambda b, d, s: (b["managing"] is False, b["managing"]))
run("start")

if os.system(f"kill -0 {engine_pid} 2>/dev/null") != 0:
    failures.append("engine exited")
timings.sort()
if timings:
    print(f"response p50={timings[len(timings)//2]:.0f} ms max={timings[-1]:.0f} ms over {len(timings)} shortcuts")
if failures:
    sys.exit(f"{len(failures)} failed: {failures}")
print("all shortcuts ok")
