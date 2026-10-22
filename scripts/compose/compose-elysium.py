"""Elysium, City of Tides: an editable native SyFrack architectural world.

The scene uses real recursive geometry, named PBR finishes and bounded detail
evaluation. There are no meshes, external textures or image-generated elements.
"""
from pathlib import Path
import argparse
import math
import random
import json

ROOT = Path(__file__).resolve().parents[2]


def num(x):
    s = f"{float(x):.8f}".rstrip("0").rstrip(".")
    return s if "." in s else s + ".0"


def vec(xs):
    return "(" + ",".join(num(x) for x in xs) + ")"


class Form:
    def __init__(self):
        self.nodes, self.links = [], []
        self.space = self.node("Input", "Espace de la cite")

    def node(self, kind, name, *sources):
        i = len(self.nodes)
        self.nodes.append(f'(id:{i}, kind:{kind}, name:"{name}", pos:{vec(((i%12)*230,(i//12)*140))}, enabled:true)')
        for port, source in enumerate(sources):
            self.links.append(f"(from:({source},0),to:({i},{port}))")
        return i

    def local(self, position, name, source=None):
        p = self.space if source is None else source
        count = max(1, math.ceil(max(abs(x) for x in position)/10))
        for _ in range(count):
            p = self.node(f"Translate(offset:{vec([-x/count for x in position])})", name, p)
        return p

    def scale(self, p, dimensions, name):
        return self.node(f"ScaleAxes(factors:{vec([1/x for x in dimensions])})", name, p)

    def material(self, d, index, name):
        return self.node(f"MaterialTag(index:{index})", name, d)

    def solid(self, p, kind, index, name):
        return self.material(self.node(kind, name, p), index, name+" - finition")

    def box(self, p, half, material, name, radius=0.08):
        return self.solid(p, f"RoundBox(half:{vec(half)},radius:{num(radius)})", material, name)

    def cylinder(self, p, radius, height, material, name):
        return self.solid(p, f"Cylinder(radius:{num(radius)},half_height:{num(height)})", material, name)

    def torus(self, p, radius, tube, material, name):
        return self.solid(p, f"Torus(radius:{num(radius)},tube:{num(tube)})", material, name)

    def union(self, branches, name):
        d = branches[0]
        for branch in branches[1:]:
            d = self.node("Union", name, d, branch)
        return d

    def group(self, p, branches, half, name):
        d = self.union(branches, name)
        return self.node(f"Bounded(half:{vec(half)},margin:0.35)", name+" - volume de detail", p, d)

    def menger(self, p, generations=4):
        body = self.node("MengerFold(scale:3.0,offset:(1.0,1.0,1.0))", "Recursion des ajours")
        loop = self.node(f"Iterate(iterations:{generations},bailout:4.0,body:[{body}])", "Dentelle de Menger", p)
        return self.node("Box(half:(1.0,1.0,1.0))", "Surface recursive", loop)

    def hybrid(self, p, generations=10):
        p = self.node("Rotate(angles:(90.0,0.0,0.0))", "Axe vertical des fleurons", p)
        body = [self.node("Power(power:8.0)", "Huit lobes des fleurons"),
                self.node("AddSeed(multiplier:(1.0,1.0,1.0))", "Reinjection locale des volumes")]
        loop = self.node(f"Iterate(iterations:{max(12,generations)},bailout:4.0,body:{body})", "Architecture Mandelbulb locale", p)
        return self.node("BulbFence", "Surface des fleurons", loop)

    def filigree(self, p, shape, material, cell=0.65, name="Filigrane"):
        q = self.node(f"Tile(period:{vec((cell,cell,cell))})", name+" - cellules", p)
        q = self.scale(q, (cell/2,)*3, name+" - finesse")
        d = self.node("Intersection", name+" - ajours reels", shape, self.menger(q,3))
        return self.material(d,material,name+" - patine")

    def island(self, x, z, radius, name, source=None, deck=4):
        p = self.local((x,0,z),name+" - implantation", source)
        parts = []
        for y, r, h, m in [(-.55,radius+.5,.95,4),(.35,radius,.25,1),(1.0,radius-.35,.4,1),(1.55,radius-.75,.15,deck)]:
            q = self.local((0,y,0),name+" - terrasses",p)
            parts.append(self.cylinder(q,r,h,m,name+" - assise"))
            parts.append(self.torus(q,r-.02,.075,2,name+" - corniche"))
        q = self.local((0,.80,0),name+" - frise",p)
        frieze = self.node(f"Torus(radius:{num(radius-.20)},tube:0.30)",name+" - frise circulaire",q)
        parts.append(self.filigree(q,frieze,3,.65,name+" - dentelle des quais"))
        return p, parts

    def dome(self, p, radius, material, name):
        q = self.scale(p,(1,.70,1),name+" - profil")
        outer = self.node(f"Sphere(radius:{num(radius)})",name+" - voute",q)
        inner = self.node(f"Sphere(radius:{num(radius-.20)})",name+" - espace interieur",q)
        d = self.node("Subtraction",name+" - coque",outer,inner)
        clip = self.node(f"Box(half:{vec((radius+1,radius+1,radius+1))})",name+" - hemisphere",self.local((0,radius+1,0),name+" - coupe",p))
        d = self.node("Intersection",name+" - hemisphere ouvert",d,clip)
        # A strong architectural skeleton remains visible at a wide camera angle.
        # Twelve meridians, several latitudes and an open crown, all geometry.
        q0 = self.node("Kaleido(planes:12,phase:0.0)",name+" - meridiens",q)
        q0 = self.node("Rotate(angles:(90.0,0.0,0.0))",name+" - nervures verticales",q0)
        rib = self.node(f"Torus(radius:{num(radius)},tube:0.065)",name+" - nervure",q0)
        rib = self.node("Intersection",name+" - demi-nervures",rib,clip)
        parts = [self.material(rib,2,name+" - bronze des nervures")]
        for latitude in [18,38,58,74]:
            angle=math.radians(latitude)
            rp=self.local((0,.70*radius*math.sin(angle),0),name+" - latitude",p)
            parts.append(self.torus(rp,radius*math.cos(angle),.052,2,name+" - ceinture"))
        # The lattice has visible apertures; small recursive relief is confined
        # to its structural strips instead of filling the whole hemisphere.
        shell = self.filigree(p,d,material,1.9,name+" - panneaux ajoures")
        openings = self.node("Kaleido(planes:12,phase:0.0)",name+" - percements",p)
        openings = self.local((radius*.60,radius*.29,0),name+" - fenetres",openings)
        openings = self.node(f"ScaleAxes(factors:{vec((1,1.2,2.4))})",name+" - profil des ouvertures",openings)
        holes = self.node(f"Sphere(radius:{num(radius*.40)})",name+" - baies",openings)
        shell = self.node("Subtraction",name+" - ciel ouvert",shell,holes)
        parts.append(shell)
        return self.union(parts,name+" - charpente et dentelle")

    def beacon(self, p, name):
        parts = [self.cylinder(p,.11,.65,2,name+" - fut")]
        q = self.local((0,.78,0),name+" - lanterne",p)
        parts.append(self.solid(q,"Sphere(radius:0.23)",5,name+" - lumiere ambree"))
        parts.append(self.torus(q,.29,.035,2,name+" - cage"))
        return parts

    def bridge(self, a, b, name, width=1.35, source=None, style=0, elevation=1.6):
        dx,dz = b[0]-a[0],b[1]-a[1]
        length = math.hypot(dx,dz)
        p = self.local(((a[0]+b[0])/2,elevation,(a[1]+b[1])/2),name+" - trajet", source)
        p = self.node(f"Rotate(angles:{vec((0,-math.degrees(math.atan2(dx,dz)),0))})",name+" - orientation",p)
        parts = [self.box(p,(width,.16 if style!=1 else .10,length/2),3 if style==1 else 1,name+" - passage")]
        q = self.node("Abs(x:true,y:false,z:false)",name+" - parapets",p)
        q = self.local((width,0.52,0),name+" - balustrades",q)
        if style==0:
            rail = self.node(f"Box(half:{vec((.12,.42,length/2))})",name+" - panneaux",q)
            parts.append(self.filigree(q,rail,2,.60,name+" - dentelle"))
        elif style==1:
            posts=self.node("Tile(period:(100.0,100.0,1.5))",name+" - potelets",q)
            post=self.cylinder(posts,.045,.48,2,name+" - montants de cuivre")
            clip=self.node(f"Box(half:{vec((.1,.5,length/2))})",name+" - limite des montants",q)
            parts.append(self.node("Intersection",name+" - garde corps",post,clip))
            parts.append(self.box(q,(.035,.035,length/2),2,name+" - lisse basse",.02))
        else:
            parts.append(self.box(q,(.12,.36,length/2),1,name+" - muret de pierre",.05))
            foundation=self.box(self.local((0,-.70,0),name+" - soubassement",p),(width,.70,length/2),1,name+" - arche porteuse")
            hollow=self.scale(self.local((0,-1.35,0),name+" - ouverture de l'arche",p),(width+1,1.35,length*.43),name+" - courbe de l'arche")
            hollow=self.node("Sphere(radius:1.0)",name+" - ouverture",hollow)
            parts.append(self.node("Subtraction",name+" - voute de pierre",foundation,hollow))
        h = self.local((0,.48,0),name+" - main courante",q)
        parts.append(self.box(h,(.13,.06,length/2),2,name+" - couronnement",.04))
        if style!=2:
            q = self.node(f"Tile(period:{vec((100,100,3.8))})",name+" - piles",p)
            q = self.local((0,-1.60,0),name+" - appuis",q)
            piles = self.cylinder(q,.22 if style==1 else .42,1.55,4,name+" - pile de basalte")
            clip = self.node(f"Box(half:{vec((width,1.66,length/2))})",name+" - emprise",self.local((0,-1.50,0),name+" - limites",p))
            parts.append(self.node("Intersection",name+" - fondations",piles,clip))
        return self.group(p,parts,(width+.35,3.25 if style!=2 else 2.3,length/2+.25),name)

    def finish(self, parts):
        d = self.union(parts,"Assembler les quartiers de la cite")
        self.node("Output","Elysium - Cite des marees",d)
        return "(nodes:[\n"+",\n".join(self.nodes)+"\n],links:[\n"+",\n".join(self.links)+"\n],safety:0.78,bounding_radius_hint:800.0)"

    def repeat(self, p, period, count, name):
        copies="("+",".join(str(n) for n in count)+")"
        return self.node(f"RepeatFinite(period:{vec(period)},count:{copies})", name, p)

    def instances(self, template, placements, half, name):
        # Keep each shared model in one dynamic GPU loop. Splitting it into
        # separate graph branches multiplies the driver's inlined fractal code.
        placements=sorted(placements,key=lambda copy:(copy[0][2],copy[0][0]))
        copies=",".join(f"(position:{vec(p)},angles:{vec(a)},scale:{vec(s)})" for p,a,s in placements)
        return self.node(f"Instances(placements:[{copies}],half:{vec(half)},margin:0.45)",name,template)


def point_segment(p,a,b):
    vx,vz=b[0]-a[0],b[1]-a[1]
    t=max(0,min(1,((p[0]-a[0])*vx+(p[1]-a[1])*vz)/(vx*vx+vz*vz)))
    return math.hypot(p[0]-a[0]-t*vx,p[1]-a[1]-t*vz)


def archipelago():
    """Seeded dart distribution; connected navigable quays, no lattice."""
    rng=random.Random(10102026)
    islands=[dict(x=0,z=-8,r=10.6,family=-1,scale=1,yaw=0),dict(x=14,z=13,r=6.7,family=-2,scale=1,yaw=0)]
    families=[0]*15+[1]*15+[2]*14+[3]*14
    rng.shuffle(families)
    radii=[6.9,7.3,7.5,6.4]
    for family in families:
        scale=rng.uniform(.86,1.12)
        radius=radii[family]*scale
        for _ in range(20000):
            x,z=rng.uniform(-73,73),rng.uniform(-184,18)
            if (x/75)**2+((z+82)/102)**2>1: continue
            if all(math.hypot(x-i['x'],z-i['z'])>radius+i['r']+rng.uniform(2.4,4.4) for i in islands):
                islands.append(dict(x=x,z=z,r=radius,family=family,scale=scale,yaw=rng.uniform(-180,180)))
                break
        else: raise RuntimeError("Archipelago placement failed")
    candidates=[]
    for i,a in enumerate(islands):
        for j in range(i+1,len(islands)):
            b=islands[j]
            distance=math.hypot(a['x']-b['x'],a['z']-b['z'])
            if distance>48: continue
            if any(point_segment((c['x'],c['z']),(a['x'],a['z']),(b['x'],b['z']))<c['r']+1.1 for k,c in enumerate(islands) if k not in (i,j)): continue
            candidates.append((distance,i,j))
    candidates.sort()
    parents=list(range(60))
    def root(i):
        while parents[i]!=i: i=parents[i]
        return i
    edges=[]
    for d,i,j in candidates:
        ri,rj=root(i),root(j)
        if ri!=rj: parents[ri]=rj; edges.append((i,j))
    if len(edges)!=59: raise RuntimeError("Archipelago network disconnected")
    def crosses(a,b,c,d):
        def side(p,q,r): return (q[0]-p[0])*(r[1]-p[1])-(q[1]-p[1])*(r[0]-p[0])
        return side(a,b,c)*side(a,b,d)<0 and side(c,d,a)*side(c,d,b)<0
    for distance,i,j in sorted(candidates,key=lambda v:v[0]*rng.uniform(.8,1.4)):
        if (i,j) in edges or distance>34: continue
        a,b=[(islands[k]['x'],islands[k]['z']) for k in (i,j)]
        if any(crosses(a,b,(islands[u]['x'],islands[u]['z']),(islands[v]['x'],islands[v]['z'])) for u,v in edges if len({i,j,u,v})==4): continue
        edges.append((i,j))
        if len(edges)==75: break
    segments=[[],[],[]]
    junctions=[]
    paths=[]
    for i,j in edges:
        a,b=islands[i],islands[j]
        dx,dz=b['x']-a['x'],b['z']-a['z']; length=math.hypot(dx,dz); dx/=length; dz/=length
        start=(a['x']+dx*(a['r']-1.15),a['z']+dz*(a['r']-1.15))
        end=(b['x']-dx*(b['r']-1.15),b['z']-dz*(b['r']-1.15))
        style=0 if i==0 or j==0 else rng.choices([0,1,2],[.25,.48,.27])[0]
        width=[1.35,.72,1.10][style]*rng.uniform(.88,1.12)
        points=[start,end]
        if math.dist(start,end)>5.5 and rng.random()<.55:
            bend=rng.uniform(-1,1)*min(3.2,math.dist(start,end)*.22)
            mid=((start[0]+end[0])/2-dz*bend,(start[1]+end[1])/2+dx*bend)
            clear=all(math.dist(mid,(c['x'],c['z']))>c['r']+width+1 for k,c in enumerate(islands) if k not in (i,j))
            if clear:
                points=[start,mid,end]
                junctions.append((mid[0]-dz*width*.65,2.4,mid[1]+dx*width*.65))
        for aa,bb in zip(points,points[1:]):
            vx,vz=bb[0]-aa[0],bb[1]-aa[1]; length=math.hypot(vx,vz)
            segments[style].append((((aa[0]+bb[0])/2,1.60,(aa[1]+bb[1])/2),(0,math.degrees(math.atan2(vx,vz)),0),(width/[1.35,.72,1.10][style],1,(length+.20)/10)))
        paths.append(dict(islands=[i,j],style=style,points=points))
    # Spread additional walkway lamps through the city, including the foreground.
    rng.shuffle(junctions)
    junctions=junctions[:15]
    return islands,segments,junctions,paths


def compose(a):
    f = Form()
    islands,segments,junctions,paths=archipelago()
    lamps=[]
    world = []
    # The ocean shader is a real displaced dielectric surface; no opaque slab.
    # Geological foundations are independent of the deliberately architectural
    # quays. Three eroded families and scattered shore boulders break their circles.
    geology = random.Random(421073)
    for family in range(3):
        rock = f.solid(f.space,f"Rock(half:(8.7,3.5,8.2),erosion:0.38,seed:{241+family*317})",4,"Roche marine erodee")
        places=[]
        for i, island in enumerate(islands):
            if i%3 != family: continue
            places.append(((island['x'],-2.0,island['z']),(0,island['yaw']+geology.uniform(-22,22),0),
                (island['r']/7,geology.uniform(.88,1.18),island['r']/7)))
        world.append(f.instances(rock,places,(10.4,4.2,9.8),"Soubassements rocheux naturels"))
    pebble=f.solid(f.space,"Rock(half:(1.0,0.8,0.92),erosion:0.42,seed:731)",4,"Blocs de basalte battus par les flots")
    boulders=[]
    for island in islands:
        for _ in range(2):
            angle=geology.uniform(0,math.tau); radius=island['r']*geology.uniform(1.02,1.16)
            size=geology.uniform(.75,1.6)*island['r']/7
            boulders.append(((island['x']+math.cos(angle)*radius,-1.35+geology.uniform(-.4,.2),island['z']+math.sin(angle)*radius),
                (geology.uniform(-15,15),geology.uniform(-180,180),geology.uniform(-12,12)),(size,size*geology.uniform(.7,1.2),size)))
    world.append(f.instances(pebble,boulders,(1.22,.98,1.13),"Eboulis des rivages"))

    # Central island: open colonnade, perforated cupola, recursive crown.
    p, parts = f.island(0,-8,10.6,"Sanctuaire des marees")
    q = f.local((0,4.70,0),"Colonnade du sanctuaire",p)
    q = f.node("Kaleido(planes:12,phase:0.0)","Douze travees",q)
    q = f.local((6.80,0,0),"Colonnes du deambulatoire",q)
    parts.append(f.cylinder(q,.38,2.95,1,"Colonnes de pierre claire"))
    for y in [-2.6,2.5]:
        cap = f.local((0,y,0),"Chapiteaux ciselés",q)
        shell = f.node("Cylinder(radius:0.73,half_height:0.30)","Chapiteaux - emprise",cap)
        parts.append(f.filigree(cap,shell,2,.26,"Chapiteaux de bronze"))
    for y,r,t,m in [(2.0,6.9,.20,2),(7.6,6.85,.32,1),(7.95,7.2,.13,2)]:
        parts.append(f.torus(f.local((0,y,0),"Entablement du temple",p),r,t,m,"Couronne du temple"))
    parts.append(f.dome(f.local((0,7.9,0),"Coupole du sanctuaire",p),7.15,3,"Coupole ajouree"))
    q = f.local((0,14.3,0),"Fleuron central",p)
    shape = f.hybrid(f.scale(q,(1.45,3.9,1.45),"Elancement du fleuron"),12)
    shape = f.material(shape,2,"Bronze du fleuron")
    parts.append(f.group(q,[shape],(2.1,5.3,2.1),"Couronne fractale du sanctuaire"))
    # Four sculptural towers anchor the sanctuary and share its hybrid grammar.
    q = f.node("Kaleido(planes:4,phase:45.0)","Quatre tours du sanctuaire",p)
    q = f.local((8.9,6.0,0),"Tours de garde",q)
    shape = f.material(f.hybrid(f.scale(q,(1.45,4.0,1.45),"Elancement des tours"),10),3,"Cuivre patine des tours")
    parts.append(f.group(q,[shape],(2.0,5.5,2.0),"Tours ornementales"))
    parts.append(f.cylinder(f.local((0,-2.2,0),"Appuis des tours",q),.38,2.2,1,"Futs porteurs des tours"))
    parts.append(f.cylinder(f.local((0,-4.25,0),"Bases des tours",q),.8,.18,2,"Socles des tours"))
    parts.append(f.cylinder(f.local((0,19.0,0),"Mat de la lanterne",p),.09,.8,2,"Mat du sommet"))
    parts.extend(f.beacon(f.local((0,19.8,0),"Lanterne du sommet",p),"Feu du sanctuaire"))
    world.append(f.group(p,parts,(11.5,21,11.5),"Sanctuaire principal"))

    # The quays share one model; upper architecture has independent height.
    # Its origin is at quay height, keeping every support attached to the deck.
    heights=[4.2,5.6,5.0,6.7]
    p,parts=f.island(0,0,7.0,"Quais des quartiers",source=f.local((0,-.1,0),"Centre vertical des quais"))
    quay=f.union(parts,"Modele commun des quais")
    placements=[((i['x'],.1,i['z']),(0,i['yaw'],0),(i['r']/7,1,i['r']/7)) for i in islands if i['family']>=0]
    world.append(f.instances(quay,placements,(7.6,1.62,7.6),"Implantation libre des quais"))
    p=f.local((0,-3.65,0),"Centre vertical des pavillons")
    parts=[]
    q=f.node("Kaleido(planes:5,phase:18.0)","Cinq pavillons de cuivre",p)
    q=f.local((3.9,3.7,0),"Pavillons des quartiers",q)
    shape=f.material(f.hybrid(f.scale(q,(1.3,2.5,1.3),"Proportions des fleurons"),12),3,"Cuivre ancien")
    parts.append(f.group(q,[shape],(1.9,3.6,1.9),"Fleurons des quartiers"))
    parts.append(f.cylinder(f.local((0,-2.05,0),"Appuis des pavillons",q),.28,1.65,1,"Colonnes porteuses"))
    parts.append(f.cylinder(f.local((0,-3.55,0),"Bases des pavillons",q),.72,.18,2,"Socles de bronze"))
    parts.append(f.cylinder(f.local((0,.55,0),"Rotondes des quartiers",p),2.15,.42,1,"Rotondes"))
    parts.append(f.dome(f.local((0,1.0,0),"Petites coupoles",p),2.1,2,"Coupoles de cuivre"))
    parts.extend(f.beacon(f.local((0,2.65,0),"Feux des quartiers",p),"Signaux des quartiers"))
    architecture=f.union(parts,"Modele des pavillons et rotondes")
    placements=[((i['x'],1.6+3.65*heights[i['family']]/5,i['z']),(0,i['yaw'],0),(i['r']/7,heights[i['family']]/5,i['r']/7)) for i in islands if i['family']>=0]
    world.append(f.instances(architecture,placements,(6.25,3.7,6.25),"Hauteurs et orientations des quartiers"))
    lamps.extend((i['x'],1.6+3.19*heights[i['family']]/5-.01,i['z'],i['family']) for i in islands if i['family']>=0)

    # Foreground terrace establishes human scale and a path into the city.
    p,parts = f.island(14,13,6.7,"Belvedere du voyageur",deck=7)
    q = f.local((0,2.35,0),"Balustrade du belvedere",p)
    outer = f.node("Cylinder(radius:5.9,half_height:0.45)","Parapet circulaire",q)
    inner = f.node("Cylinder(radius:5.6,half_height:0.60)","Vide du parapet",q)
    rail = f.node("Subtraction","Parapet de dentelle",outer,inner)
    # Open the parapet on the side that meets the bridge.
    doorway = f.node("Box(half:(1.7,0.8,3.0))","Acces au pont",f.local((-2.4,0,-4.9),"Entree du belvedere",q))
    rail = f.node("Subtraction","Passage du voyageur",rail,doorway)
    parts.append(f.filigree(q,rail,2,.55,"Balustrade du voyageur"))
    parts.append(f.torus(f.local((0,.48,0),"Main courante du belvedere",q),5.75,.095,2,"Couronnement de bronze"))
    # Intricate astronomical instrument, situated off centre to preserve the view.
    q = f.local((2.6,3.0,.6),"Astrolabe des marées",p)
    parts.append(f.cylinder(f.local((0,-.7,0),"Socle de l'astrolabe",q),.9,.45,4,"Socle de basalte"))
    for angle,r,t in [(0,1.20,.055),(58,1.05,.05),(112,.82,.045)]:
        ring = f.node(f"Rotate(angles:{vec((angle,0,25))})","Plans de l'astrolabe",q)
        parts.append(f.torus(ring,r,t,2,"Anneaux de navigation"))
    parts.append(f.solid(q,"Sphere(radius:0.20)",5,"Coeur lumineux de l'astrolabe"))
    q = f.node("Kaleido(planes:6,phase:0.0)","Lanternes du belvedere",p)
    parts.extend(f.beacon(f.local((5.1,2.8,0),"Implantation des lanternes",q),"Lanternes du voyageur"))
    world.append(f.group(p,parts,(7.6,5.0,7.6),"Terrasse du premier plan"))

    # A connected network uses three distinct architectural bridge families.
    for style,(name,width) in enumerate([("Ponts ceremoniels",1.35),("Passerelles de cuivre",.72),("Viaducs de pierre",1.1)]):
        template=f.bridge((0,-5),(0,5),name,width=width,style=style,elevation=0)
        world.append(f.instances(template,segments[style],(width+.4,3.3 if style!=2 else 2.4,5.3),name+" - parcours des canaux"))
    if junctions:
        template=f.union(f.beacon(f.space,"Lanternes des passerelles"),"Modele de lanterne")
        world.append(f.instances(template,[(p,(0,0,0),(1,1,1)) for p in junctions],(.4,1.1,.4),"Lumieres des croisements"))
        lamps.extend((x,y+.54,z,4) for x,y,z in junctions)
    for k in range(6):
        angle=2*math.pi*k/6
        lamps.append((14+5.1*math.cos(angle),3.34,13+5.1*math.sin(angle),5))
    # Local illumination at the sanctuary entrance and below its open cupola.
    lamps.extend([(0,9.5,-8,6),(0,20.34,-8,6),(16.6,2.7,13.6,5)])

    graph=f.finish(world)
    print(f"Native composition: 60 irregular islands, {len(paths)} connections, {sum(map(len,segments))} bridge segments, {len(lamps)+3} lights, {len(f.nodes)} nodes")
    metadata=ROOT/Path(a.out).with_suffix('.layout.json')
    metadata.parent.mkdir(parents=True,exist_ok=True)
    metadata.write_text(json.dumps(dict(seed=10102026,islands=islands,paths=paths,light_count=len(lamps)+3),indent=2),encoding="utf-8")
    finishes = [
        ("Limestone",(.62,.54,.40),.49,.02,(0,0,0)),
        ("Warm bronze",(.48,.24,.075),.28,.88,(0,0,0)),
        ("Verdigris copper",(.055,.24,.205),.35,.64,(0,0,0)),
        ("Coastal basalt",(.17,.19,.18),.68,0.0,(0,0,0)),
        ("Amber lantern glass",(.45,.18,.025),.20,.05,(18.0,5.6,.95)),
        ("Distant weathered stone",(.22,.29,.285),.56,.10,(0,0,0)),
    ]
    finishes.append(("Engraved tidal pavement",(.085,.095,.09),.42,.32,(0,0,0)))
    details = [
        ("VeinedStone",17,2.5,.35,.004,.12,(.30,.25,.17)),
        ("Patina",31,1.8,.38,.003,.20,(.04,.14,.12)),
        ("Patina",47,2.8,.55,.008,.24,(.045,.10,.075)),
        ("CoastalRock",59,3.6,.62,.018,.20,(.065,.08,.07)),
        ("BrushedMetal",67,1.0,.12,.001,.06,(.60,.28,.06)),
        ("VeinedStone",73,1.4,.28,.005,.15,(.12,.16,.15)),
        ("Rosettes",83,.75,.85,-.025,-.10,(.35,.19,.06)),
    ]
    palette_parts=[]
    for (n,c,r,m,e),(pattern,seed,scale,strength,bump,rough,tint) in zip(finishes,details):
        detail=f"Some((pattern:{pattern},seed:{seed},scale:{num(scale)},strength:{num(strength)},bump:{num(bump)},roughness_delta:{num(rough)},tint:{vec(tint)}))"
        specular=.04
        palette_parts.append(f'(name:"{n}",albedo:{vec(c)},roughness:{num(r)},metallic:{num(m)},specular:{num(specular)},emission:{vec(e)},detail:{detail})')
    palette = ",\n".join(palette_parts)
    lighting=",\n".join(f"Area(position:{vec((x,y,z))},normal:(0.0,-1.0,0.0),size:(0.42,0.42),color:Kelvin({num(2600+family*110)}),intensity:{num(80 if family==6 else 45)})" for x,y,z,family in lamps)
    return f'''// ELYSIUM - CITY OF TIDES. Native recursive geometry; fully editable.
(
 seed:10102026,fractal:Graph({graph}),
 camera:(mode:Free(position:{vec(a.camera)},target:{vec(a.target)},roll:0.0),focal_mm:{num(a.focal)},f_number:0.0,focus_distance:55.0),
 lights:[
  Sun(direction:(-0.65,0.14,0.35),color:Kelvin(4800.0),intensity:0.35,angular_size:1.0),
  Area(position:(85.0,155.0,100.0),normal:(-0.4,-0.7,-0.59),size:(42.0,42.0),color:Kelvin(7500.0),intensity:2.3),
  Environment(zenith:Linear((0.09,0.13,0.20)),horizon:Linear((0.18,0.23,0.30)),ground:Linear((0.04,0.05,0.065)),intensity:0.75,rotation:0.0),
  {lighting}
 ],
 material:(albedo:(0.60,0.60,0.60),roughness:0.45,metallic:0.0,specular:0.04,coloring:None,texture:None,surface:None,finishes:[{palette}]),
 atmosphere:(fog_density:{num(a.fog)},fog_color:(0.12,0.18,0.30),fog_height:8.0,fog_falloff:0.06,anisotropy:0.35,cavity_darkness:0.04,
  seascape:Some((sea_level:-1.55,sky_zenith:(0.06,0.11,0.18),sky_horizon:(0.19,0.24,0.29),water_color:(0.045,0.14,0.19),intensity:0.7,wave_strength:0.06,wave_scale:0.6,clouds:0.20,haze:0.16,
   surface:Some((wave_height:0.65,wavelength:7.8,wind_direction:-32.0,crest_sharpness:0.65,time:8.0,ripple_strength:0.32,roughness:0.105,foam_amount:0.56,shore_width:0.36,depth:12.0,ior:1.333,absorption:(0.20,0.055,0.028),scattering:0.035,foam_color:(0.86,0.92,0.94),seabed_color:(0.11,0.15,0.14)))))),
 look:(exposure_ev:1.9,auto_exposure:None,temperature:6500.0,tint:0.0,
  shadows:(offset:(0.0,0.0,0.0),gain:0.0),midtones:(offset:(0.0,0.0,0.0),gain:0.0),highlights:(offset:(0.0,0.0,0.0),gain:0.0),
  tonemap:Agx(slope:1.0,power:1.06,offset:0.0),saturation:1.04,vibrance:0.08,vignette:Some((radius:1.0,softness:1.0,intensity:0.10,elliptical:true)),grain:None,output:Srgb,dither:true)
)
'''


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out",default="scenes/environments/elysium.ron")
    parser.add_argument("--camera",nargs=3,type=float,default=[52,85,48])
    parser.add_argument("--target",nargs=3,type=float,default=[0,0,-38])
    parser.add_argument("--focal",type=float,default=25)
    parser.add_argument("--fog",type=float,default=.0004)
    args=parser.parse_args()
    out=ROOT/args.out
    out.parent.mkdir(parents=True,exist_ok=True)
    out.write_text(compose(args),encoding="utf-8")
    print(out)
