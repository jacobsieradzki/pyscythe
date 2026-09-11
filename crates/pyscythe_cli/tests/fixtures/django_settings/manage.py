import os

from shop.admin import OrderAdmin
from shop.managers import UserManager

os.environ.setdefault("DJANGO_SETTINGS_MODULE", "config.django.base")
print(OrderAdmin, UserManager)
