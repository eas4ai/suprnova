// S3 exposes custom response headers as user metadata. Restore the validated public delivery headers.
function handler(event) {
    var response = event.response;
    var headers = response.headers;
    for (var key in headers) {
        if (key.indexOf('x-amz-meta-') === 0) {
            var name = key.substring('x-amz-meta-'.length);
            headers[name] = { value: headers[key].value };
            delete headers[key];
        }
    }
    var type = headers['content-type'];
    if (type && type.value.indexOf('text/html') === 0 && !headers['content-security-policy']) {
        headers['content-security-policy'] = { value: "default-src 'none'; sandbox" };
    }
    return response;
}
