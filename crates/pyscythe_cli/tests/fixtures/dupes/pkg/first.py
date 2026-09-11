def summarise_orders(orders):
    total = 0
    count = 0
    biggest = None
    for order in orders:
        if order.cancelled:
            continue
        total += order.amount
        count += 1
        if biggest is None or order.amount > biggest.amount:
            biggest = order
    if count == 0:
        return None
    return {"total": total, "count": count, "biggest": biggest, "average": total / count}
