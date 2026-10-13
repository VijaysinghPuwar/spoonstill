"""Kill `still move` / `still picture --to` / `still remove` at random moments
on a numbered project and check that recovery never loses, duplicates or
mispairs a file (D-201), and never splits a removed scene (D-203). Fetch the 431-scene fixture with scripts/stress.sh first.

usage: python3 -I scripts/kill-stress.py STILL FIXTURE_DIR WORK_DIR ROUNDS
       SEED=N to vary the schedule; a round is one command, killed or not.
"""
import hashlib, os, random, re, shutil, subprocess, sys, time
from collections import Counter

still, fixture, work, rounds = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
# Windows' CreateProcess does not find a relative `target/release/still` the
# way a unix exec does, so the documented invocation is resolved here
still = os.path.abspath(still)
if os.name == "nt" and not still.lower().endswith(".exe") and os.path.isfile(still + ".exe"):
    still += ".exe"
random.seed(int(os.environ.get("SEED", "7")))
IMG = {"jpg", "jpeg", "png"}

def h(p):
    with open(p, "rb") as f:
        return hashlib.sha1(f.read()).hexdigest()[:12]

def snapshot(d):
    """{stem: {'pic': [hash..], 'txt': [hash..]}} and a list of stray names."""
    scenes, stray = {}, []
    for name in os.listdir(d):
        p = os.path.join(d, name)
        if not os.path.isfile(p):
            continue
        if name.startswith("."):
            if name != ".DS_Store":
                stray.append(name)
            continue
        m = re.fullmatch(r"(\d+)\.(\w+)", name)
        if not m:
            continue
        stem, ext = m.group(1), m.group(2).lower()
        kind = "pic" if ext in IMG else "txt" if ext == "txt" else None
        if kind:
            scenes.setdefault(int(stem), {"pic": [], "txt": []})[kind].append(h(p))
    return scenes, stray

def binned(d):
    """Hashes of everything in removed/, which a removal moves files into."""
    b = os.path.join(d, "removed")
    return Counter(h(os.path.join(b, n)) for n in os.listdir(b)) if os.path.isdir(b) else Counter()

def check(d, before, mode, label, bin_before=Counter(), gone=None):
    # validate only reports an interrupted rename; an arrange command repairs it
    rec = subprocess.run([still, "move", d, "1", "1"], capture_output=True, text=True)
    if rec.returncode != 0:
        print(f"FAIL {label}: recovery command failed: {(rec.stdout + rec.stderr).strip()[-300:]}", flush=True)
        return False
    # and a second repair must be a no-op (idempotent)
    mid, _ = snapshot(d)
    subprocess.run([still, "move", d, "1", "1"], capture_output=True)
    if snapshot(d)[0] != mid:
        print(f"FAIL {label}: second recovery changed the project", flush=True)
        return False
    run =subprocess.run([still, "validate", "--no-probe", d], capture_output=True, text=True)
    after, stray = snapshot(d)
    errs = []
    if stray:
        errs.append(f"parked files left after recovery: {stray[:5]}")
    for sid, s in after.items():
        if len(s["pic"]) != 1:
            errs.append(f"scene {sid} has {len(s['pic'])} pictures")
        if len(s["txt"]) != 1:
            errs.append(f"scene {sid} has {len(s['txt'])} narrations")
    if sorted(after) != list(range(1, len(after) + 1)):
        errs.append("scene numbers have a gap")
    pics_b = Counter(x for s in before.values() for x in s["pic"])
    pics_a = Counter(x for s in after.values() for x in s["pic"])
    txt_b = Counter(x for s in before.values() for x in s["txt"])
    txt_a = Counter(x for s in after.values() for x in s["txt"])
    pair = lambda S: Counter((tuple(s["pic"]), tuple(s["txt"])) for s in S.values())
    if mode == "remove":
        # a removal moves files into removed/: count the bin with the project
        if pics_a + txt_a + binned(d) != pics_b + txt_b + bin_before:
            errs.append("a file was lost or duplicated across the project and removed/")
        whole = pair(before) - Counter([(tuple(gone["pic"]), tuple(gone["txt"]))])
        if pair(after) not in (pair(before), whole):
            errs.append("a removed scene was split, or another scene lost its partner")
    elif pics_a != pics_b:
        errs.append("picture set changed (lost or duplicated)")
    if mode != "remove" and txt_a != txt_b:
        errs.append("narration set changed")
    if mode == "move":
        if pair(after) != pair(before):
            errs.append("a picture was separated from its narration")
    elif mode == "picture":  # narration order never changes
        if {k: v["txt"] for k, v in after.items()} != {k: v["txt"] for k, v in before.items()}:
            errs.append("a narration moved during a picture move")
    if run.returncode != 0:
        errs.append("validate failed: " + (run.stdout + run.stderr).strip()[-300:])
    if errs:
        print(f"FAIL {label}: " + "; ".join(errs[:6]), flush=True)
        return False
    return True

os.makedirs(work, exist_ok=True)
proj = os.path.join(work, "kill")
if os.path.exists(proj):
    shutil.rmtree(proj)
os.makedirs(proj)
for name in sorted(os.listdir(fixture)):
    if re.fullmatch(r"\d+\.(jpg|txt)", name):
        dst = name
        # every fifth picture is a .png, so swaps mix extensions (D-200)
        if name.endswith(".jpg") and int(name[:3]) % 5 == 0:
            dst = name[:-4] + ".png"
        # `cp -c` is a macOS clone and Windows has no `cp`; a plain copy is the
        # same bytes everywhere, which is all the harness compares
        shutil.copyfile(os.path.join(fixture, name), os.path.join(proj, dst))

n = len(snapshot(proj)[0])

def timed(argv):
    t = time.monotonic()
    subprocess.run(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    return time.monotonic() - t

# A kill only tests recovery if it lands while files are being renamed, and how
# long that takes is the machine's: a 431-scene move is tens of milliseconds on
# an APFS laptop and ~0.6 s on NTFS. Each command's own duration is measured
# (there and back, so the project is unchanged) and kills are spread across it.
window = {
    "move": max(timed([still, "move", proj, str(n), "1"]), timed([still, "move", proj, "1", str(n)])),
    "picture": max(timed([still, "picture", proj, "2", "--to", str(n - 1)]),
                   timed([still, "picture", proj, str(n - 1), "--to", "2"])),
}
print(f"kill windows: move {window['move']:.3f} s, picture {window['picture']:.3f} s", flush=True)

def parked(d):
    return any(name.startswith((".arranging", ".removing-")) for name in os.listdir(d))

fails = kills = interrupted = 0
for r in range(rounds):
    # a removal is rarer, so a long run does not empty the project
    mode = random.choices(["move", "picture", "remove"], [4, 4, 1])[0]
    n = len(snapshot(proj)[0])
    a, b = random.sample(range(1, n + 1), 2)
    argv = ([still, "move", proj, str(a), str(b)] if mode == "move"
            else [still, "remove", proj, str(a)] if mode == "remove"
            else [still, "picture", proj, str(a), "--to", str(b)])
    before, _ = snapshot(proj)
    bin_before = binned(proj)
    p = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(random.uniform(0, window["move" if mode == "remove" else mode]))
    if p.poll() is None:
        # SIGKILL on unix, TerminateProcess on Windows, where the signal module
        # has no SIGKILL at all: both stop the process with no chance to clean up
        p.kill()
        kills += 1
    p.wait()
    # a kill that left a parked file stopped a rename part-way, which is the
    # case under test; one that left none hit startup or the finished command
    interrupted += parked(proj)
    gone = before.get(a) if mode == "remove" else None
    if not check(proj, before, mode, f"round {r} {' '.join(argv[1:2] + argv[3:])}", bin_before, gone):
        fails += 1
        if fails >= 5:
            break
print(f"done: {r + 1} rounds, {kills} killed mid-run, {interrupted} left a rename part-way, {fails} failures", flush=True)
