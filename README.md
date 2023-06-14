# Cecs 🍪

Cecs, pronounced [ˈkɛks] is an ECS implementation supporting Cao-Lo.

There are many like it, but this one is mine.

Heavily inspired by [Bevy](https://bevyengine.org/) and [Hexops](https://devlog.hexops.com/2022/lets-build-ecs-part-2-databases/)

*Minimum supported Rust version is 1.72*

## Features

- Functions as systems
- Query interface
- Unique data, called Resources
- Cloning of the entire database (optional)
- Serialization/Deserialization of select _components_ (optional)
- (Very basic) parallel scheduler (optional)
