// S3 exposes custom response headers as user metadata. Restore only public delivery headers.
function handler(event) {
    var response = event.response;
    var headers = response.headers;
    var allowed = [
        'content-security-policy', 'content-security-policy-report-only',
        'x-content-type-options', 'referrer-policy', 'permissions-policy',
        'x-frame-options', 'strict-transport-security', 'cross-origin-opener-policy',
        'cross-origin-resource-policy', 'cross-origin-embedder-policy', 'x-inertia', 'etag'
    ];
    for (var i = 0; i < allowed.length; i++) {
        var name = allowed[i];
        var metadata = headers['x-amz-meta-' + name];
        if (metadata) headers[name] = { value: metadata.value };
    }
    for (var key in headers) {
        if (key.indexOf('x-amz-meta-') === 0) delete headers[key];
    }
    var type = headers['content-type'];
    if (type && type.value.indexOf('text/html') === 0 && !headers['content-security-policy']) {
        headers['content-security-policy'] = { value: "default-src 'none'; sandbox" };
    }
    return response;
}
