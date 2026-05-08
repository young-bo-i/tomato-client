"""
One-shot MySQL (legacy koltool @ 10.0.0.120) -> local Docker Postgres migration.

READ-ONLY on the MySQL side. Skips the huge task/submit tables per user spec:
  kolbrushtask, kolbrushnontask, QiMaoBrushTask, QiMaoBrushNonTask,
  submitbrushrequest, submitwordstatistics

Run inside a container attached to the `server_internal` network so that
`postgres:5432` resolves. MySQL is reached via host IP 10.0.0.120.
"""
import os
import sys
from datetime import datetime
import pymysql
import psycopg

MYSQL_CFG = dict(
    host=os.environ.get("MYSQL_HOST", "10.0.0.120"),
    port=int(os.environ.get("MYSQL_PORT", 3306)),
    user=os.environ.get("MYSQL_USER", "root"),
    password=os.environ.get("MYSQL_PASSWORD", "123456"),
    database=os.environ.get("MYSQL_DB", "koltool"),
    charset="utf8mb4",
    cursorclass=pymysql.cursors.DictCursor,
    read_timeout=300,
)

PG_DSN = os.environ.get(
    "PG_DSN",
    "host=postgres port=5432 dbname=tomato_kol user=tomato password=tomato123",
)


def as_bool(v):
    """MySQL stores StdIsDeleted/IsHasChildAccount as int 0/1. Map to bool."""
    if v is None:
        return False
    try:
        return int(v) != 0
    except (TypeError, ValueError):
        return bool(v)


def parse_dt(v):
    if v is None or v == "":
        return None
    if isinstance(v, datetime):
        return v
    # QiMaoAccount.ProSignTime is a varchar in source
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y/%m/%d %H:%M:%S", "%Y-%m-%d"):
        try:
            return datetime.strptime(str(v), fmt)
        except ValueError:
            continue
    return None


SCENE_STR = {1: "KolOpenTypes", 2: "KolLimit", 5: "QiMaoOpenTypes", 6: "QiMaoLimit"}


def migrate(mysql_conn, pg_conn):
    with mysql_conn.cursor() as mc, pg_conn.cursor() as pc:
        # ---- account ----
        print("[account] loading...")
        mc.execute("SELECT * FROM account")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO account (id, account_name, password_hash, phone, email,
                       status, name, validity_time, is_deleted, created_at, updated_at)
                   VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountName"], r["Password"] or "",
                    r["Phone"], r["Email"], int(r["Status"] or 1),
                    r["Name"], parse_dt(r["ValidityTime"]),
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                    r["ModifiedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[account] {len(rows)} inserted")

        # ---- kol_account (from kolcookies) ----
        print("[kol_account] loading...")
        mc.execute("SELECT * FROM kolcookies")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO kol_account (id, account_id, cookies, storage_state,
                       uid, identity_name, identity_number, payment_account, mobile,
                       audit_time, remark, status, is_deleted, created_at, updated_at)
                   VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountId"], r["Cookies"], r["StorageState"],
                    r["UId"], r["IdentityName"], r["IdentityNumber"],
                    r["PaymentAccount"], r["Mobile"], parse_dt(r["AuditTime"]),
                    r["Remark"], 1,
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                    r["ModifiedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[kol_account] {len(rows)} inserted")

        # ---- douyin_account (from douyincookies) ----
        print("[douyin_account] loading...")
        mc.execute("SELECT * FROM douyincookies")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO douyin_account (id, account_id, storage_state, nickname,
                       douyin_uid, remark, status, follow_count, fan_count,
                       is_deleted, created_at, updated_at)
                   VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountId"], r["StorageState"], r["NickName"],
                    r["DouYinId"], r["Remark"],
                    int(r["Status"] or 1),
                    int(r["FollowCount"] or 0), int(r["FanCount"] or 0),
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                    r["ModifiedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[douyin_account] {len(rows)} inserted")

        # ---- qimao_account ----
        print("[qimao_account] loading...")
        mc.execute("SELECT * FROM QiMaoAccount")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO qimao_account (id, account_id, remark, identifier,
                       credential, token, status, nickname, phone_no, user_id, type,
                       taxpayer_type, pro_phone_no, pro_id_card_no, pro_real_name,
                       pro_account_no, pro_sign_status, pro_verify_status, pro_sign_time,
                       bank_name, bank_province, bank_city, back_account_no,
                       bank_address, bank_phone_no, is_deleted, created_at, updated_at)
                   VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountId"], r["Remark"], r["Identifier"],
                    r["Credential"], r["Token"], int(r["Status"] or 1),
                    r["Nickname"], r["PhoneNo"], r["UserId"],
                    int(r["Type"] or 0), int(r["TaxpayerType"] or 0),
                    r["ProPhoneNo"], r["ProIdCardNo"], r["ProRealName"],
                    r["ProAccountNo"], r["ProSignStatus"],
                    int(r["ProVerifyStatus"] or 0), parse_dt(r["ProSignTime"]),
                    r["BankName"], r["BankProvince"], r["BankCity"],
                    r["BackAccountNo"], r["BankAddress"], r["BankPhoneNo"],
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                    r["ModifiedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[qimao_account] {len(rows)} inserted")

        # ---- account_secret_key ----
        print("[account_secret_key] loading...")
        mc.execute("SELECT * FROM accountsecretkeys")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO account_secret_key (id, account_id, secret_key, remark,
                       status, is_deleted, created_at, updated_at)
                   VALUES (%s,%s,%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountId"], r["SecretKey"] or "",
                    r["Remark"], int(r["Status"] or 1),
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                    r["ModifiedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[account_secret_key] {len(rows)} inserted")

        # ---- kol_invite_code ----
        print("[kol_invite_code] loading...")
        mc.execute("SELECT * FROM Kolinvitecode")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO kol_invite_code (id, account_id, kol_id, invite_code,
                       share_token, x_kol_token, is_deleted, created_at, updated_at)
                   VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountId"] or 0, r["KolId"] or 0,
                    r["InviteCode"] or "", r["ShareToken"], r["XKolToken"],
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                    r["ModifiedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[kol_invite_code] {len(rows)} inserted")

        # ---- kol_book ----
        print("[kol_book] loading...")
        mc.execute("SELECT * FROM kolbook")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO kol_book (id, book_id, book_name, platform,
                       is_deleted, created_at)
                   VALUES (%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["TomatoBookId"] or "", r["BookName"] or "",
                    int(r["Platform"] or 1),
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[kol_book] {len(rows)} inserted")

        # ---- qimao_book ----
        print("[qimao_book] loading...")
        mc.execute("SELECT * FROM QiMaoBook")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO qimao_book (id, book_id, book_name, is_forbidden,
                       is_deleted, created_at)
                   VALUES (%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], str(r["QiMaoBookId"] or ""), r["BookName"] or "",
                    False,
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[qimao_book] {len(rows)} inserted")

        # ---- kol_income ----
        print("[kol_income] loading...")
        mc.execute("SELECT * FROM kolincome")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO kol_income (id, account_id, kol_id, total_income,
                       regular_income, bonus_income, current_month_income,
                       current_week_income, income_json, monthly_income_list_json,
                       weekly_income_list_json, last_update_time, created_at, updated_at)
                   VALUES (%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountId"] or 0, r["KolId"] or 0,
                    int(r["TotalIncome"] or 0), int(r["RegularIncome"] or 0),
                    int(r["BonusIncome"] or 0), int(r["CurrentMonthIncome"] or 0),
                    int(r["CurrentWeekIncome"] or 0),
                    r["IncomeJson"], r["MonthlyIncomeListJson"],
                    r["WeeklyIncomeListJson"],
                    r["LastUpdateTime"] or datetime.utcnow(),
                    r["CreatedTime"] or datetime.utcnow(),
                    r["ModifiedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[kol_income] {len(rows)} inserted")

        # ---- income_notice ----
        print("[income_notice] loading...")
        mc.execute("SELECT * FROM incomenotice")
        rows = mc.fetchall()
        for r in rows:
            pc.execute(
                """INSERT INTO income_notice (id, account_id, email, has_child,
                       is_deleted, created_at)
                   VALUES (%s,%s,%s,%s,%s,%s)""",
                (
                    r["Id"], r["AccountId"] or 0, r["ToEmail"] or "",
                    as_bool(r["IsHasChildAccount"]),
                    as_bool(r["StdIsDeleted"]),
                    r["CreatedTime"] or datetime.utcnow(),
                ),
            )
        print(f"[income_notice] {len(rows)} inserted")

        # ---- common_setting + dom_config ----
        print("[common_setting + dom_config] loading...")
        mc.execute("SELECT * FROM commonsetting")
        rows = mc.fetchall()
        cs_n = dom_n = 0
        for r in rows:
            scene = int(r["Scene"] or 0)
            value = r["Value"] or ""
            if scene in (1, 2):  # KOL settings
                pc.execute(
                    """INSERT INTO common_setting (id, account_id, kol_id, scene,
                           setting_value, is_deleted, created_at, updated_at)
                       VALUES (%s,%s,%s,%s,%s,%s,%s,%s)""",
                    (
                        r["Id"], r["AccountId"] or 0, r["KolId"] or 0,
                        SCENE_STR[scene], value,
                        as_bool(r["StdIsDeleted"]),
                        r["CreatedTime"] or datetime.utcnow(),
                        r["ModifiedTime"] or datetime.utcnow(),
                    ),
                )
                cs_n += 1
            elif scene in (5, 6):  # QiMao settings — reuse kol_id column for QiMaoId
                pc.execute(
                    """INSERT INTO common_setting (id, account_id, kol_id, scene,
                           setting_value, is_deleted, created_at, updated_at)
                       VALUES (%s,%s,%s,%s,%s,%s,%s,%s)""",
                    (
                        r["Id"], r["AccountId"] or 0, r["QiMaoId"] or 0,
                        SCENE_STR[scene], value,
                        as_bool(r["StdIsDeleted"]),
                        r["CreatedTime"] or datetime.utcnow(),
                        r["ModifiedTime"] or datetime.utcnow(),
                    ),
                )
                cs_n += 1
            elif scene == 3:  # DouYin DOM
                pc.execute(
                    """INSERT INTO dom_config (dom_type, selectors, updated_at)
                       VALUES ('douyin', %s::jsonb, %s)
                       ON CONFLICT (dom_type) DO UPDATE
                         SET selectors = EXCLUDED.selectors, updated_at = EXCLUDED.updated_at""",
                    (value, r["ModifiedTime"] or datetime.utcnow()),
                )
                dom_n += 1
            elif scene == 4:  # Kol DOM
                pc.execute(
                    """INSERT INTO dom_config (dom_type, selectors, updated_at)
                       VALUES ('kol', %s::jsonb, %s)
                       ON CONFLICT (dom_type) DO UPDATE
                         SET selectors = EXCLUDED.selectors, updated_at = EXCLUDED.updated_at""",
                    (value, r["ModifiedTime"] or datetime.utcnow()),
                )
                dom_n += 1
        print(f"[common_setting] {cs_n} inserted, [dom_config] {dom_n} upserted")

        # ---- permission (merge: keep new-server seeds, add legacy ones) ----
        print("[permission] loading...")
        mc.execute("SELECT * FROM permission")
        rows = mc.fetchall()
        for r in rows:
            if not r["Code"]:
                continue
            pc.execute(
                """INSERT INTO permission (code, name) VALUES (%s,%s)
                   ON CONFLICT (code) DO NOTHING""",
                (r["Code"], r["Name"] or r["Code"]),
            )
        print(f"[permission] {len(rows)} processed (merged)")

        # ---- user_auth (lookup permission_id by code) ----
        print("[user_auth] loading...")
        mc.execute("SELECT * FROM userAuth")
        rows = mc.fetchall()
        skipped = 0
        inserted = 0
        for r in rows:
            code = r["PermissionCode"]
            uid = r["UserId"]
            if not code or not uid:
                skipped += 1
                continue
            pc.execute("SELECT id FROM permission WHERE code = %s", (code,))
            pid = pc.fetchone()
            if not pid:
                skipped += 1
                continue
            pc.execute(
                """INSERT INTO user_auth (account_id, permission_id) VALUES (%s, %s)
                   ON CONFLICT (account_id, permission_id) DO NOTHING""",
                (uid, pid[0]),
            )
            inserted += 1
        print(f"[user_auth] {inserted} inserted, {skipped} skipped")

        # ---- reset sequences to max(id)+1 ----
        print("[sequences] resetting...")
        seq_tables = [
            "account", "kol_account", "douyin_account", "qimao_account",
            "account_secret_key", "kol_invite_code", "kol_book", "qimao_book",
            "kol_income", "income_notice", "common_setting", "permission", "user_auth",
        ]
        for t in seq_tables:
            pc.execute(
                f"""SELECT setval(pg_get_serial_sequence('{t}', 'id'),
                       COALESCE((SELECT MAX(id) FROM {t}), 1), true)"""
            )
        print("[sequences] done")

    pg_conn.commit()
    print("\nCOMMITTED.")


def main():
    print("connecting to MySQL...")
    mc = pymysql.connect(**MYSQL_CFG)
    print("connecting to Postgres...")
    pc = psycopg.connect(PG_DSN)
    try:
        migrate(mc, pc)
    except Exception:
        pc.rollback()
        raise
    finally:
        mc.close()
        pc.close()


if __name__ == "__main__":
    sys.exit(main())
