<?php

namespace App\Notifications;

use Illuminate\Notifications\Notification;

// Sent through the database channel only, so it writes one notifications row.
class InvoicePaid extends Notification
{
    public function __construct(public int $invoiceId)
    {
    }

    public function via(object $notifiable): array
    {
        return ['database'];
    }

    public function toArray(object $notifiable): array
    {
        return [
            'invoice_id' => $this->invoiceId,
            'amount' => 99.5,
        ];
    }
}
