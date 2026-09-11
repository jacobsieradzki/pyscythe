def summarise_invoices(invoices):
    sum_ = 0
    n = 0
    largest = None
    for invoice in invoices:
        if invoice.cancelled:
            continue
        sum_ += invoice.amount
        n += 1
        if largest is None or invoice.amount > largest.amount:
            largest = invoice
    if n == 0:
        return None
    return {"total": sum_, "count": n, "biggest": largest, "average": sum_ / n}


def unrelated(x):
    return x * 2
