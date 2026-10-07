"""Kill `still move` / `still picture --to` at random moments on a numbered
project and check that recovery never loses, duplicates or mispairs a file
(D-201). Fetch the 431-scene fixture with scripts/stress.sh first.

usage: python3 -I scripts/kill-stress.py STILL FIXTURE_DIR WORK_DIR ROUNDS
       SEED=N to vary the schedule; a round is one command, killed or not.
"""
import hashlib, os, random, re, shutil, signal, subprocess, sys, time
from collections import Counter

still, fixture, work, rounds = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
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

def check(d, before, mode, label):
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
    if pics_a != pics_b:
        errs.append("picture set changed (lost or duplicated)")
    if txt_a != txt_b:
        errs.append("narration set changed")
    if mode == "move":
        pair = lambda S: Counter((tuple(s["pic"]), tuple(s["txt"])) for s in S.values())
        if pair(after) != pair(before):
            errs.append("a picture was separated from its narration")
    else:  # picture --to: narration order never changes
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
        subprocess.run(["cp", "-c", os.path.join(fixture, name), os.path.join(proj, dst)], check=True)

n = len(snapshot(proj)[0])
fails = kills = 0
for r in range(rounds):
    mode = random.choice(["move", "picture"])
    a, b = random.sample(range(1, n + 1), 2)
    argv = ([still, "move", proj, str(a), str(b)] if mode == "move"
            else [still, "picture", proj, str(a), "--to", str(b)])
    before, _ = snapshot(proj)
    p = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(random.uniform(0, 0.06 if mode == "move" else 0.02))
    if p.poll() is None:
        p.send_signal(signal.SIGKILL)
        kills += 1
    p.wait()
    if not check(proj, before, mode, f"round {r} {' '.join(argv[1:2] + argv[3:])}"):
        fails += 1
        if fails >= 5:
            break
print(f"done: {r + 1} rounds, {kills} killed mid-run, {fails} failures", flush=True)
