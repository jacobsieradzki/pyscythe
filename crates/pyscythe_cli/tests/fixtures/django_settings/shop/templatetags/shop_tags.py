from django.template import Library

register = Library()


@register.filter
def money(value: int) -> str:
    return f"{value}"


@register.simple_tag
def badge() -> str:
    return ""
