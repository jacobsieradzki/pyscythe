"""Every constant here is consumed by the loop at the bottom."""

NAME = 1
NUMBER = 2
BACKQUOTE = 25

tok_name: dict[int, str] = {}
for _name, _value in list(globals().items()):
    if type(_value) is int:
        tok_name[_value] = _name
