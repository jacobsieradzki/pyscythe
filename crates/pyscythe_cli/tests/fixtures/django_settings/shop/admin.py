from django.contrib import admin


class OrderAdmin(admin.ModelAdmin):
    list_display = ["qr_code", "total"]

    def qr_code(self, obj) -> str:
        return "qr"

    def total(self, obj) -> int:
        return 1

    def forgotten(self) -> None:
        pass
