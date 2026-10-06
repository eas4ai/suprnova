<?php

namespace App\Models;

use Illuminate\Database\Eloquent\Model;
use Illuminate\Database\Eloquent\Relations\MorphMany;
use Illuminate\Database\Eloquent\Relations\MorphOne;
use Illuminate\Database\Eloquent\SoftDeletes;

// A soft-deleting model with a json column and two polymorphic children.
class Post extends Model
{
    use SoftDeletes;

    protected $fillable = ['title', 'meta'];

    protected function casts(): array
    {
        return [
            'meta' => 'array',
        ];
    }

    public function comments(): MorphMany
    {
        return $this->morphMany(Comment::class, 'commentable');
    }

    public function image(): MorphOne
    {
        return $this->morphOne(Image::class, 'imageable');
    }
}
