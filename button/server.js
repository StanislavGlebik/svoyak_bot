var http = require('http');
var fs = require('fs');

const EventEmitter = require('events');

class ClientEmitter extends EventEmitter {}
const clientEmitter = new ClientEmitter();
clientEmitter.on('event', () => {
  console.log('a client event occurred!');
});


class ServerEmitter extends EventEmitter {}
const serverEmitter = new ServerEmitter();
serverEmitter.on('event', () => {
  console.log('a server event occurred!');
});

let currentstate = "stopped";
function togglestate() {
    if (currentstate == "stopped") {
        currentstate = "started";
    } else {
        currentstate = "stopped";
    }
}

http.createServer(function (req, res) {
  console.log("path " + req.url)
  if (req.url == "/") {
      try {
          const data = fs.readFileSync("./user.html", 'utf8');
          res.writeHead(200, {'Content-Type': 'text/html'});
          res.end(data)
      } catch (err) {
          res.writeHead(503, {'Content-Type': 'text/html'});
          res.end("server failure - failed to read html file\n" + err);
      }
  } else if (req.url == "/admin") {
      try {
          const data = fs.readFileSync("./admin.html", 'utf8');
          res.writeHead(200, {'Content-Type': 'text/html'});
          res.end(data)
      } catch (err) {
          res.writeHead(503, {'Content-Type': 'text/html'});
          res.end("server failure - failed to read html file\n" + err);
      }
  } else if (req.url.startsWith("/adminlongpoll")) {
      serverEmitter.once('event', (data) => {
        console.log("calling event")
        res.writeHead(200, {'Content-Type': 'text/plain'});
        res.end(data);
      });
  } else if (req.url == "/buttonpressed") {
      let data = '';
      req.on('data', chunk => {
        data += chunk;
      })
      req.on('end', () => {
        console.log(data);
        serverEmitter.emit('event', data);
        res.end()
      })
  } else if (req.url.startsWith("/clientgetcurrentstate")) {
      console.log("getting current state")
      res.writeHead(200, {'Content-Type': 'text/plain'});
      res.end(currentstate);
  } else if (req.url.startsWith("/clientlongpoll")) {

      clientEmitter.once('event', () => {
        console.log("calling event")
        res.writeHead(200, {'Content-Type': 'text/plain'});
        res.end(currentstate);
      });
      console.log("finished /clientlongpoll");
  } else if (req.url == "/emit") {
    togglestate();
    clientEmitter.emit('event');
    res.end();
  } else {
      res.writeHead(404, {'Content-Type': 'text/html'});
      res.end()
  }
}).listen(8080);
