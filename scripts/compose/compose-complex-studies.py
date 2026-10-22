"""A reproducible native SyFrack salon: classical complex dynamics and artistic hybrids."""
import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("volutes", Path(__file__).with_name("compose-volutes-chromatiques.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

PALETTES = {
    "iridescence": [(.006,.012,.024),(.004,.04,.23),(.005,.28,.34),(.012,.62,.66),(.86,.74,.47),(.78,.34,.08),(.70,.03,.01),(.78,.09,.05),(.33,.007,.06),(.018,.009,.03)],
    "nacre": [(.002,.008,.025),(.01,.045,.16),(.004,.36,.60),(.40,.80,.77),(.86,.87,.82),(.69,.52,.19),(.025,.17,.20),(.005,.01,.03)],
    "cuivre": [(.018,.007,.015),(.32,.006,.018),(.75,.012,.009),(.86,.20,.04),(.92,.57,.15),(.85,.75,.53),(.018,.20,.25),(.003,.04,.08)],
}

STUDIES = [
    dict(name="01-julia-azur",family="Julia",julia=(-.74543,.11301),center=(-.15,.05),zoom=2.8,iterations=192,stripes=17,frequency=20,weights=(1,.03,.12,0),palette="nacre"),
    dict(name="02-julia-soie",family="Julia",julia=(-.8,.156),center=(.08,.02),zoom=5,angle=23,iterations=192,stripes=8,frequency=13,weights=(1,.08,.18,0)),
    dict(name="03-mandelbrot-volutes",family="Mandelbrot",center=(-.7435,.1314),zoom=240,iterations=256,stripes=17,frequency=21,weights=(1,.08,.10,0)),
    dict(name="04-mandelbrot-or",family="Mandelbrot",center=(-.76,.1),zoom=24,iterations=192,stripes=13,frequency=21,weights=(1,.05,.15,0),palette="cuivre"),
    dict(name="05-burning-ship-soie",family="BurningShip",center=(-1.76,-.035),zoom=120,iterations=192,stripes=11,frequency=17,weights=(1,.12,.07,0),palette="nacre"),
    dict(name="06-burning-ship-cuivre",family="BurningShip",center=(-1.75,-.02),zoom=36,iterations=192,stripes=8,frequency=13,weights=(1,.15,.12,0),palette="cuivre"),
    dict(name="07-newton-petales",family="Newton",power=3,zoom=1.4,iterations=96,stripes=24,trap_radius=.33,frequency=8,weights=(.12,.28,.60,.05)),
    dict(name="08-newton-spirales",family="Newton",power=3,zoom=1.2,angle=23,inversion=.75,relaxation=.9,twist=17,iterations=128,stripes=13,trap_radius=.8,frequency=8,weights=(.15,.32,.52,.05),palette="cuivre"),
    dict(name="09-nova-velours",family="Nova",julia=(-.16,.18),power=3,zoom=2.4,relaxation=.95,twist=6,iterations=128,stripes=17,trap_radius=.75,frequency=8,weights=(.1,.16,.64,.1)),
    dict(name="10-nova-tissage",family="Nova",julia=(.21,-.05),power=4,center=(.12,-.05),zoom=1.1,inversion=.55,relaxation=.83,twist=24,iterations=192,stripes=21,frequency=8,weights=(.14,.21,.59,.06),palette="nacre"),
    dict(name="11-phoenix-argent",family="Phoenix",julia=(.56667,0),memory=-.5,zoom=2.8,iterations=192,stripes=13,frequency=13,weights=(1,.12,.13,0),palette="nacre"),
    dict(name="12-phoenix-dragon",family="Phoenix",julia=(-.2,.65),memory=-.35,center=(-.15,.04),zoom=1.6,inversion=.55,twist=8,iterations=128,stripes=21,frequency=11,weights=(.6,.15,.20,.05)),
    dict(name="13-nova-moire",family="Nova",julia=(.2,.4),power=3,zoom=1.2,inversion=.8,relaxation=.75,twist=17,iterations=192,stripes=34,frequency=8,weights=(.18,.27,.50,.05)),
    dict(name="14-julia-spirale",family="Julia",julia=(-.7,.26),zoom=2,inversion=.75,twist=9,iterations=192,stripes=21,frequency=13,weights=(.65,.1,.20,.05)),
    dict(name="15-newton-corolle",family="Newton",power=5,center=(.1,0),zoom=1.2,inversion=.8,relaxation=.85,twist=24,iterations=160,stripes=13,frequency=7,weights=(.1,.24,.56,.10),palette="cuivre"),
    dict(name="16-nova-nacre",family="Nova",julia=(.12,.07),power=3,center=(.05,.02),zoom=3.1,relaxation=.9,twist=-18,iterations=192,stripes=27,frequency=11,weights=(.1,.27,.56,.07),palette="nacre"),
    dict(name="17-julia-sillage",family="Julia",julia=(-.7,.26),zoom=2,inversion=.75,twist=9,offset=(2.4,-.4,0),iterations=192,stripes=13,frequency=13,weights=(.9,.08,.12,0)),
    dict(name="18-nova-baroque",family="Nova",julia=(.2,.4),power=3,center=(.1,0),zoom=1.2,inversion=.8,relaxation=.75,twist=17,offset=(2.4,-.3,0),iterations=144,stripes=8,bailout=64,frequency=5,weights=(.2,.4,.4,0)),
    dict(name="19-newton-couture",family="Newton",power=5,center=(.1,0),zoom=1.2,inversion=.8,relaxation=.85,twist=24,offset=(2.4,.2,0),iterations=144,stripes=5,bailout=64,frequency=4,weights=(.2,.45,.35,0),palette="cuivre"),
    dict(name="20-julia-glace",family="Julia",julia=(-.74543,.11301),center=(-.15,.05),zoom=2.8,iterations=192,stripes=17,frequency=13,weights=(.8,.15,.18,0),palette="nacre"),
    dict(name="21-nova-brocart",family="Nova",julia=(.2,.4),power=3,center=(-.642120809,-.131538869),zoom=40,relaxation=.75,twist=17,iterations=192,stripes=13,bailout=64,frequency=8,weights=(.18,.27,.5,0)),
    dict(name="22-newton-filigrane",family="Newton",power=5,center=(-.693828078,-.205029880),zoom=30,relaxation=.85,twist=24,iterations=192,stripes=13,bailout=64,frequency=8,weights=(.15,.3,.5,0),palette="cuivre"),
]

def vector(v):
    return "("+",".join(str(float(x)) for x in v)+")"

def compose(study):
    defaults = dict(power=2,iterations=128,julia=(-.74543,.11301),center=(0,0),zoom=1,angle=0,inversion=0,twist=0,relaxation=1,memory=-.5,bailout=16,trap_radius=.5,stripes=12)
    settings = defaults | {k:v for k,v in study.items() if k in defaults}
    parameters = ",".join(f"{k}:{vector(v) if isinstance(v,tuple) else v}" for k,v in settings.items())
    f = module.Form()
    sphere = f.node("Sphere(radius:1.0)","Source de texture inactive en mode complexe",f.space)
    graph = f.finish([sphere]).replace("bounding_radius_hint:800.0","bounding_radius_hint:4.0")
    colours = PALETTES[study.get("palette","iridescence")]
    stops = "["+",".join(vector(c) for c in colours)+"]"
    base = module.compose(1)
    a = base.index("texture:Some(")
    b = base.index("surface:None",a)
    texture = f'''texture:Some((graph:{graph},planar:Some((family:{study['family']},{parameters})),
channel:Orbit(channels:(traps:{vector(study['weights'])},iterations:0.0,normal:0.0,logarithmic:0.0)),
scale:.65,offset:{vector(study.get("offset",(0,0,0)))},rotation:(0.0,0.0,0.0),octaves:1,contrast:1.0,
ramp:(stops:{stops},cyclic:true,frequency:{study['frequency']},phase:.17,gamma:1.0,interpolation:Smooth),
strength:1.0,roughness:0.0,bump:.018,parallax:0.0,parallax_steps:8,fan:None,mask:None)),'''
    base = base[:a]+texture+base[b:]
    base = base.replace("roughness:.48,metallic:0.25","roughness:.55,metallic:.35").replace("intensity:5.0","intensity:3.0").replace("intensity:.7","intensity:.45").replace("exposure_ev:.1","exposure_ev:-.1").replace("saturation:1.18","saturation:1.3")
    return "// "+study["name"]+" · original native SyFrack complex study\n"+base

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out",default="scenes/studies/complex-salon-20261013")
    parser.add_argument("--select",help="One study name, otherwise the whole salon")
    args = parser.parse_args()
    folder = ROOT/args.out
    folder.mkdir(parents=True,exist_ok=True)
    selected = [s for s in STUDIES if not args.select or s["name"]==args.select]
    if not selected: raise SystemExit("Unknown study")
    for study in selected:
        (folder/(study["name"]+".ron")).write_text(compose(study),encoding="utf-8")
    (folder/"studies.json").write_text(json.dumps([s for s in STUDIES if (folder/(s["name"]+".ron")).exists()],indent=2)+"\n",encoding="utf-8")
    print(f"{len(selected)} native studies in {folder}")
