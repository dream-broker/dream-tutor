# DrEAM TuTor
The build server for DreamMaker. **USE AT YOUR OWN RISK.**

## Usage
- Start the server
- Proxy all requests from DreamMaker to `http://YOUR_SERVER_IP:3000`
- Login account by using `xyxx` as both username and password
- Build your game as normal

## Dependencies
LuaJIT v2.0.5 is required before build. Read the documentation of [mlua](https://github.com/khvzak/mlua#compiling) for how to setup in detail.

## Caution
Some options do not work for now and the building history will be discarded after each time application stop.
