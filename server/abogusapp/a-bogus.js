require("./env")
require("./bdms")
const http = require('http');
const {Buffer} = require('buffer');
function get_a_bogus(params, body, ua) {
    arguments = [
        0,
        1,
        14,
        params,
        body,
        ua
    ]
    let r = window.dy._v;

    let re = (0, window.dy._u)(r[0], arguments, r[1], r[2], this);
    window.dy._v = r;
    return re;
}

// const result = get_a_bogus("params", "body", "ua");
const server = http.createServer((req, res) => {
    if (req.method === 'POST' && req.url === '/api/get-a-bogus') {
        let body = '';

        // 接收请求体
        req.on('data', chunk => {
            body += chunk.toString();
        });

        req.on('end', () => {
            try {
                const { params, body: requestBody, ua } = JSON.parse(body);

                if (typeof params === 'undefined' || typeof ua === 'undefined') {
                    res.writeHead(400, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({ error: 'Missing parameters' }));
                    return;
                }

                const result = get_a_bogus(params, requestBody, ua);
                res.writeHead(200, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ result }));
            } catch (err) {
                res.writeHead(400, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ error: 'Invalid JSON' }));
            }
        });
    } else {
        res.writeHead(404, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Not Found' }));
    }
});

const port = 3000;
server.listen(port, () => {
    console.log(`Server is running at http://localhost:${port}`);
});
