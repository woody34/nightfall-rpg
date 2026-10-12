"""Certify integer-angle comparisons over the finite test-zone coordinate domain.

No trig library: Machin's identity and alternating Taylor bounds use Fraction.
A squared cosine interval containing no rational with denominator <= 4*C**4
cannot contain any reachable dot**2 / (norm_f**2*norm_v**2).
"""
from fractions import Fraction as F
from math import factorial
from functools import lru_cache


def atan_inverse(q, terms):
    partial = sum(((-1)**k * F(1, (2*k+1)*q**(2*k+1)) for k in range(terms)), F())
    other = partial + (-1)**terms * F(1, (2*terms+1)*q**(2*terms+1))
    return min(partial, other), max(partial, other)


@lru_cache(maxsize=None)
def pi_interval(terms):
    a,b = atan_inverse(5,terms);c,d = atan_inverse(239,terms)
    lo,hi=16*a-4*d,16*b-4*c
    scale=1<<256
    return F(lo*scale//1,scale),F(-((-hi*scale)//1),scale)


def cosine_interval(degrees, terms=40):
    # Reflection reduces argument to [0, pi/2], where cosine is decreasing.
    degrees = F(degrees) % 360
    if degrees > 180: degrees = 360-degrees
    sign = 1
    if degrees > 90: degrees = 180-degrees;sign = -1
    if not degrees:return F(sign),F(sign)
    if degrees == 90:return F(0),F(0)
    lo,hi = pi_interval(terms)
    lo*=degrees/180;hi*=degrees/180
    def bounds(x):
        total = sum(((-1)**k*x**(2*k)/factorial(2*k) for k in range(terms)),F())
        other = total + (-1)**terms*x**(2*terms)/factorial(2*terms)
        return min(total,other),max(total,other)
    lower,upper = bounds(hi)[0],bounds(lo)[1]
    return (lower,upper) if sign==1 else (-upper,-lower)


def simplest_fraction(lower, upper):
    """Smallest-denominator rational in the inclusive interval (continued fractions)."""
    if lower > upper: raise ValueError('inverted interval')
    whole = lower.numerator // lower.denominator
    if lower == whole:return F(whole)
    if upper >= whole+1:return F(whole+1)
    return whole + 1/simplest_fraction(1/(upper-whole),1/(lower-whole))


def fraction_string(value):
    return f'{value.numerator}/{value.denominator}'


def generate_angles(bound=256000):
    maximum_denominator=4*bound**4
    special={0:F(1),60:F(3,4),90:F(1,2),120:F(1,4),180:F(0),240:F(1,4),270:F(1,2),300:F(3,4),360:F(1)}
    result=[]
    for degrees in range(361):
        lower,upper=cosine_interval(F(degrees,2))
        # Store manageable enclosing dyadics, then prove their separation.
        bits=192;scale=1<<bits
        lo=F(lower*scale//1,scale);hi=F(-((-upper*scale)//1),scale)
        if degrees in special:
            square_lo=square_hi=special[degrees]
            certified=True;exact='rational_squared_boundary'
        else:
            full_lo,full_hi=cosine_interval(degrees)
            square_lo=F(((1+full_lo)/2)*scale//1,scale)
            square_hi=F(-((-((1+full_hi)/2)*scale)//1),scale)
            certified=simplest_fraction(square_lo,square_hi).denominator > maximum_denominator
            exact=''
        result.append({'degrees':degrees,'special':exact,'cos_lower':fraction_string(lo),'cos_upper':fraction_string(hi),
            'cos2_lower':fraction_string(square_lo),'cos2_upper':fraction_string(square_hi),'certified':certified,
            'blockers':[] if certified else ['angle:uncertified:'+str(degrees)]})
    return {'schema':1,'sources':[], 'coordinate_bound_millitiles':bound,
            'proof':'Machin alternating pi; cosine Taylor interval; minimal denominator > 4*coordinate_bound^4; exact rational special angles',
            'angles':result}
