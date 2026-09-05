from pkg.widget import Child, Registry, Widget


def run(thing, widget: Widget) -> None:
    print(widget.render(), widget.describe(1))
    print(thing.duck_typed())
    print(getattr(widget, "reflected")())


print(Registry(), Child())
