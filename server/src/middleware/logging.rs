use actix_web::middleware::Logger;

pub fn request_logger() -> Logger {
    Logger::new("%a \"%r\" %s %b \"%{Referer}i\" %T")
}
