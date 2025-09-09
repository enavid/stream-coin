run:
cargo run --bin stream-coin

containerd:
sudo nerdctl compose -f ./docker-compose.yml up

docker:
sudo docker compose up

swagger:
http://localhost:8080/swagger-ui/#/

kafka-ui:
http://localhost:8083/ui/

redis insight:
http://localhost:8082/

for debug:
use log::debug;
debug!("Payload: {}", payload);


todo:
- create output variable
- fix redis insight