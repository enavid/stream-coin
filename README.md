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

---
https://wallex.ir/api-document
https://docs.bitpin.ir/v1/docs/Introduction/bitpin-api-documentation
https://docs.abantether.com/#buy-orders-list
https://docs.ramzinex.com/#tag/()/operation/currenciesId
https://apidocs.exir.io/#introduction
https://apidocs.nobitex.ir/#0f4b9b52e8
https://docs.tetherland.com/docs/tetherland/15fd2beb3a6d3-get-user-info
https://docs.tabdeal.org/#00d56275ee
https://docs.bit24.cash/#api-24
---

todo:
- create output variable
- fix redis insight
