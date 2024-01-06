{
  dockerTools,
  shelve,
}:
dockerTools.buildLayeredImage {
  name = "icewind1991/shelve";
  tag = "latest";
  maxLayers = 5;
  contents = [
    shelve
    dockerTools.caCertificates
  ];
  config = {
    Cmd = ["shelve"];
    ExposedPorts = {
      "80/tcp" = {};
    };
    Env = [
      "ROCKET_ADDRESS=0.0.0.0"
      "ROCKET_PORT=80"
    ];
  };
}
