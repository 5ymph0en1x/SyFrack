"""Native SyFrack orbit-coloured study inspired by flowing, chromatic fractal petals."""
import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("syfrack_form", Path(__file__).with_name("compose-elysium.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
Form = module.Form


def compose(variant=1):
    surface = Form()
    plane = surface.node("Plane(normal:(0.0,0.0,1.0),offset:0.0)", "Support de la gravure fractale", surface.space)
    shape = surface.finish([plane]).replace("bounding_radius_hint:800.0", "bounding_radius_hint:6.0").replace("Elysium - Cite des marees", "Volutes chromatiques")
    f = Form()
    q = f.node("ScaleAxes(factors:(0.72,1.45,1.0))", "Etirement des petales", f.space)
    q = f.node("Bend(rate:0.32)", "Courant des volutes", q)
    if variant >= 3:
        q = f.node("BoxFoldAxes(limit:(0.65,0.85,1.0),x:true,y:true,z:false)", "Repli des grands eventails", q)
    q = f.node("Translate(offset:(0.34,-0.22,0.0))", "Decentrement du tourbillon", q)
    q = f.node("SphereInversion(radius:1.25,min_radius:0.10)", "Grande inversion des eventails", q)
    q = f.node("Rotate(angles:(0.0,0.0,-21.0))", "Diagonale du courant", q)
    body = [
        f.node("Translate(offset:(-0.43,0.20,0.0))", "Centre de chaque courbe"),
        f.node("SphereInversion(radius:1.05,min_radius:0.075)", "Inversion recursive"),
        f.node("Rotate(angles:(0.0,0.0,9.0))", "Rotation entre generations"),
        f.node("Scale(factor:1.13)", "Ouverture des petales"),
        f.node("AddSeed(multiplier:(0.16,0.16,0.0))", "Reinjection de la graine locale"),
        f.node("Trap(slot:0,measure:Norm,scale:3.0,mode:Minimum)", "Coeur des orbites"),
        f.node("Trap(slot:1,measure:LogNorm,scale:4.0,mode:Last)", "Finesse logarithmique"),
        f.node("Trap(slot:2,measure:AngleXY,scale:1.0,mode:Last)", "Angle final de l orbite"),
        f.node("Trap(slot:3,measure:PlaneY,scale:1.2,mode:Minimum)", "Liseres des petales"),
    ] if variant != 5 else [
        f.node("Trap(slot:0,measure:Norm,scale:1.4,mode:Minimum)", "Courbes du coeur de Julia"),
        f.node("Trap(slot:1,measure:LogNorm,scale:4.0,mode:Maximum)", "Amplitude de l orbite"),
        f.node("Trap(slot:2,measure:AngleXY,scale:1.0,mode:Minimum)", "Angle minimal des petales"),
        f.node("Trap(slot:3,measure:PlaneY,scale:1.0,mode:Minimum)", "Filigrane de Julia"),
        f.node("Power(power:3.0)", "Puissance cubique des petales"),
        f.node("AddConstant(multiplier:(0.0,0.0,0.0),julia:(-0.13,0.65,0.0))", "Constante de Julia"),
    ]
    iterations, bailout = (21, 4.0) if variant == 5 else ((8, 100.0) if variant >= 6 else (13, 100.0))
    if variant == 2:
        body.insert(0, f.node("BoxFoldAxes(limit:(0.85,0.9,1.0),x:true,y:true,z:false)", "Plis continus des eventails"))
    loop = f.node(f"Iterate(iterations:{iterations},bailout:{bailout},body:{body})", "Generations des volutes", q)
    end = f.node("NormFence(inset:0.0)", "Champ de texture fractale", loop)
    texture = f.finish([end]).replace("bounding_radius_hint:800.0", "bounding_radius_hint:6.0").replace("Elysium - Cite des marees", "Orbites des volutes")
    # Linear RGB palette: dark blue and narrow gold breaks separate luminous colour fields.
    colours = [
        (.004,.012,.07), (.015,.11,.44), (.015,.65,.82), (.05,.95,.87),
        (.75,.98,.82), (.94,.59,.20), (.91,.23,.10), (.88,.055,.045),
        (.85,.012,.16), (.60,.018,.35), (.23,.018,.31), (.007,.04,.13),
        (.035,.34,.40), (.20,.57,.18), (.88,.91,.20), (.95,.70,.42),
    ]
    if variant >= 4:
        texture = texture.replace("measure:AngleXY,scale:1.0,mode:Last", "measure:AngleXY,scale:1.0,mode:Minimum").replace("measure:LogNorm,scale:4.0,mode:Last", "measure:LogNorm,scale:4.0,mode:Maximum").replace("Angle final de l orbite", "Angle minimal de l orbite")
    frequency = 9.0 if variant == 1 else (34.0 if variant == 3 else 21.0)
    if variant == 5: frequency = 5.5
    if variant >= 6: frequency = 3.0
    rotation = "(90.0,0.0,11.0)" if variant >= 6 else "(0.0,0.0,0.0)"
    fan = "Some((period:(3.6,2.4),rays:13))" if variant >= 6 else "None"
    angle_weight = .20 if variant < 3 else (8.0/34.0 if variant == 3 else 5.0/21.0)
    log_gain = 1.5 if variant < 3 else 0.0
    metal = 0.25 if variant == 1 else (0.4 if variant >= 5 else 0.75)
    weights = (.68,.04,.11,-.03) if variant == 5 else (.28,.15,angle_weight,-.16)
    saturation = 1.18 if variant == 1 else 1.65
    points = "[" + ",".join(str(c).replace(" ", "") for c in colours) + "]"
    return f'''// VOLUTES CHROMATIQUES - native recursive orbit texture, no bitmap assets.
(
seed:13102026,fractal:Graph({shape}),
camera:(mode:Free(position:(0.0,0.0,6.0),target:(-.30,.08,0.0),roll:0.0),focal_mm:36.0,f_number:0.0,focus_distance:6.0),
lights:[
Area(position:(-2.0,3.0,5.0),normal:(.32,-.48,-.81),size:(5.0,5.0),color:Linear((1.0,.95,.86)),intensity:5.0),
Environment(zenith:Linear((.80,.86,1.0)),horizon:Linear((.80,.86,1.0)),ground:Linear((.80,.86,1.0)),intensity:.7,rotation:0.0)],
material:(albedo:(.95,.95,.95),roughness:.48,metallic:{metal},specular:.04,coloring:None,
texture:Some((graph:{texture},
channel:Orbit(channels:(traps:{str(weights).replace(" ", "")},iterations:.09,normal:0.0,logarithmic:{log_gain})),
scale:.80,offset:(.18,-.06,0.0),rotation:{rotation},octaves:1,contrast:1.0,
ramp:(stops:{points},cyclic:true,frequency:{frequency},phase:.12,gamma:1.0,interpolation:Linear),
strength:1.0,roughness:.05,bump:.07,parallax:0.0,parallax_steps:8,fan:{fan},mask:None)),surface:None,finishes:[]),
atmosphere:(fog_density:0.0,fog_color:(1.0,1.0,1.0),fog_height:0.0,fog_falloff:1.0,anisotropy:0.0,cavity_darkness:0.0,seascape:None),
look:(exposure_ev:.1,auto_exposure:None,temperature:6500.0,tint:0.0,
shadows:(offset:(0.0,0.0,0.0),gain:0.0),midtones:(offset:(0.0,0.0,0.0),gain:0.0),highlights:(offset:(0.0,0.0,0.0),gain:0.0),
tonemap:Agx(slope:1.0,power:1.06,offset:0.0),saturation:{saturation},vibrance:.08,vignette:None,grain:None,output:Srgb,dither:true)
)
'''


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--variant", type=int, default=1)
    parser.add_argument("--out", default="scenes/studies/volutes-chromatiques.ron")
    args = parser.parse_args()
    path = ROOT / args.out
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(compose(args.variant), encoding="utf-8")
    path.with_suffix(".math.json").write_text(json.dumps({"title":"Volutes chromatiques", "iterations":21 if args.variant>=5 else 13, "palette_stops":16, "cyclic_frequency":5.5 if args.variant>=5 else (9 if args.variant==1 else (34 if args.variant==3 else 21)), "geometry":"plane with native recursive fractal texture and filtered shading relief", "orbit_channels":["minimum norm","maximum logarithmic norm" if args.variant>=4 else "last logarithmic norm","minimum XY angle" if args.variant>=4 else "last XY angle","minimum plane distance"], "reference":"visual inspiration from the user's attached fractal; original formula unknown"},indent=2)+"\n",encoding="utf-8")
    print(path)
