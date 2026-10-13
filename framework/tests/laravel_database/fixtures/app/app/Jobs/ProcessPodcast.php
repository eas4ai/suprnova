<?php

namespace App\Jobs;

use Illuminate\Bus\Batchable;
use Illuminate\Contracts\Queue\ShouldQueue;
use Illuminate\Foundation\Queue\Queueable;

// A queued job that is never run while the fixtures are built: its rows stay
// in the jobs table as Laravel wrote them.
class ProcessPodcast implements ShouldQueue
{
    use Batchable, Queueable;

    public function __construct(public int $podcastId)
    {
    }

    public function handle(): void
    {
    }
}
