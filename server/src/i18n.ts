// The pages' text in the app's languages, chosen from the browser's
// Accept-Language header. "Unlock" and "Activate" match the app's buttons in
// each language (src/i18n.rs).

export type Texts = {
  incompleteTitle: string;
  incompleteBody: string;
  notPaidTitle: string;
  notPaidBody: string;
  thanksTitle: string;
  /** `{email}`, `{unlock}` and `{activate}` are filled in. */
  thanksBody: string;
  unlock: string;
  activate: string;
  copy: string;
  copied: string;
};

const en: Texts = {
  incompleteTitle: 'That link is incomplete',
  incompleteBody: 'Open the link from your payment confirmation again.',
  notPaidTitle: 'Payment not found',
  notPaidBody: 'This checkout has not been paid. If you were charged, reply to your receipt email and we will sort it out.',
  thanksTitle: 'Thank you! Here is your licence key',
  thanksBody: 'Licensed to {email}. In the app choose {unlock}, paste the key and press {activate}. Bookmark this page: it always shows your key.',
  unlock: 'Unlock',
  activate: 'Activate',
  copy: 'Copy key',
  copied: 'Copied',
};

export const texts: Record<string, Texts> = {
  en,
  de: {
    incompleteTitle: 'Dieser Link ist unvollständig',
    incompleteBody: 'Öffnen Sie den Link aus Ihrer Zahlungsbestätigung erneut.',
    notPaidTitle: 'Zahlung nicht gefunden',
    notPaidBody: 'Dieser Kauf wurde nicht bezahlt. Falls Ihnen etwas berechnet wurde, antworten Sie auf die E-Mail mit Ihrem Beleg, und wir klären das.',
    thanksTitle: 'Vielen Dank! Hier ist Ihr Lizenzschlüssel',
    thanksBody: 'Lizenziert für {email}. Wählen Sie in der App {unlock}, fügen Sie den Schlüssel ein und klicken Sie auf {activate}. Speichern Sie diese Seite als Lesezeichen: Sie zeigt immer Ihren Schlüssel.',
    unlock: 'Freischalten',
    activate: 'Aktivieren',
    copy: 'Schlüssel kopieren',
    copied: 'Kopiert',
  },
  fr: {
    incompleteTitle: 'Ce lien est incomplet',
    incompleteBody: 'Rouvrez le lien depuis votre confirmation de paiement.',
    notPaidTitle: 'Paiement introuvable',
    notPaidBody: "Cet achat n'a pas été payé. Si vous avez été débité, répondez à l'e-mail de votre reçu et nous réglerons le problème.",
    thanksTitle: 'Merci ! Voici votre clé de licence',
    thanksBody: "Licence accordée à {email}. Dans l'application, choisissez {unlock}, collez la clé et cliquez sur {activate}. Ajoutez cette page à vos favoris : elle affiche toujours votre clé.",
    unlock: 'Débloquer',
    activate: 'Activer',
    copy: 'Copier la clé',
    copied: 'Copié',
  },
  es: {
    incompleteTitle: 'Ese enlace está incompleto',
    incompleteBody: 'Vuelve a abrir el enlace de tu confirmación de pago.',
    notPaidTitle: 'Pago no encontrado',
    notPaidBody: 'Esta compra no se ha pagado. Si se te ha cobrado, responde al correo de tu recibo y lo resolveremos.',
    thanksTitle: '¡Gracias! Aquí tienes tu clave de licencia',
    thanksBody: 'Licencia para {email}. En la aplicación elige {unlock}, pega la clave y pulsa {activate}. Guarda esta página en marcadores: siempre muestra tu clave.',
    unlock: 'Desbloquear',
    activate: 'Activar',
    copy: 'Copiar clave',
    copied: 'Copiada',
  },
  it: {
    incompleteTitle: 'Questo link è incompleto',
    incompleteBody: 'Riapri il link dalla conferma di pagamento.',
    notPaidTitle: 'Pagamento non trovato',
    notPaidBody: "Questo acquisto non è stato pagato. Se ti è stato addebitato un importo, rispondi all'email della ricevuta e risolveremo.",
    thanksTitle: 'Grazie! Ecco la tua chiave di licenza',
    thanksBody: "Licenza per {email}. Nell'app scegli {unlock}, incolla la chiave e premi {activate}. Aggiungi questa pagina ai preferiti: mostra sempre la tua chiave.",
    unlock: 'Sblocca',
    activate: 'Attiva',
    copy: 'Copia chiave',
    copied: 'Copiata',
  },
  pt: {
    incompleteTitle: 'Esse link está incompleto',
    incompleteBody: 'Abra novamente o link da sua confirmação de pagamento.',
    notPaidTitle: 'Pagamento não encontrado',
    notPaidBody: 'Esta compra não foi paga. Se você foi cobrado, responda ao e-mail do recibo e resolveremos.',
    thanksTitle: 'Obrigado! Aqui está sua chave de licença',
    thanksBody: 'Licenciado para {email}. No aplicativo, escolha {unlock}, cole a chave e clique em {activate}. Salve esta página nos favoritos: ela sempre mostra sua chave.',
    unlock: 'Desbloquear',
    activate: 'Ativar',
    copy: 'Copiar chave',
    copied: 'Copiada',
  },
  nl: {
    incompleteTitle: 'Die link is onvolledig',
    incompleteBody: 'Open de link uit je betalingsbevestiging opnieuw.',
    notPaidTitle: 'Betaling niet gevonden',
    notPaidBody: 'Deze aankoop is niet betaald. Is er toch geld afgeschreven, beantwoord dan de e-mail met je bon en we lossen het op.',
    thanksTitle: 'Bedankt! Hier is je licentiesleutel',
    thanksBody: 'Licentie voor {email}. Kies in de app {unlock}, plak de sleutel en klik op {activate}. Maak een bladwijzer van deze pagina: je sleutel staat hier altijd.',
    unlock: 'Ontgrendelen',
    activate: 'Activeren',
    copy: 'Sleutel kopiëren',
    copied: 'Gekopieerd',
  },
  pl: {
    incompleteTitle: 'Ten link jest niepełny',
    incompleteBody: 'Otwórz ponownie link z potwierdzenia płatności.',
    notPaidTitle: 'Nie znaleziono płatności',
    notPaidBody: 'Ten zakup nie został opłacony. Jeśli pobrano opłatę, odpowiedz na e-mail z potwierdzeniem, a wyjaśnimy sprawę.',
    thanksTitle: 'Dziękujemy! Oto Twój klucz licencyjny',
    thanksBody: 'Licencja dla {email}. W aplikacji wybierz {unlock}, wklej klucz i kliknij {activate}. Dodaj tę stronę do zakładek: zawsze pokazuje Twój klucz.',
    unlock: 'Odblokuj',
    activate: 'Aktywuj',
    copy: 'Kopiuj klucz',
    copied: 'Skopiowano',
  },
  tr: {
    incompleteTitle: 'Bu bağlantı eksik',
    incompleteBody: 'Ödeme onayınızdaki bağlantıyı yeniden açın.',
    notPaidTitle: 'Ödeme bulunamadı',
    notPaidBody: 'Bu satın alma ödenmemiş. Sizden ücret alındıysa makbuz e-postanızı yanıtlayın, sorunu çözelim.',
    thanksTitle: 'Teşekkürler! Lisans anahtarınız burada',
    thanksBody: '{email} adına lisanslandı. Uygulamada {unlock} seçeneğini seçin, anahtarı yapıştırın ve {activate} düğmesine basın. Bu sayfayı yer imlerine ekleyin: anahtarınızı her zaman gösterir.',
    unlock: 'Kilidi aç',
    activate: 'Etkinleştir',
    copy: 'Anahtarı kopyala',
    copied: 'Kopyalandı',
  },
  ru: {
    incompleteTitle: 'Ссылка неполная',
    incompleteBody: 'Откройте ссылку из подтверждения оплаты ещё раз.',
    notPaidTitle: 'Платёж не найден',
    notPaidBody: 'Эта покупка не оплачена. Если с вас списали деньги, ответьте на письмо с чеком, и мы разберёмся.',
    thanksTitle: 'Спасибо! Вот ваш лицензионный ключ',
    thanksBody: 'Лицензия для {email}. В приложении нажмите {unlock}, вставьте ключ и нажмите {activate}. Добавьте эту страницу в закладки: на ней всегда показан ваш ключ.',
    unlock: 'Разблокировать',
    activate: 'Активировать',
    copy: 'Скопировать ключ',
    copied: 'Скопировано',
  },
  ja: {
    incompleteTitle: 'リンクが不完全です',
    incompleteBody: 'お支払い確認のリンクをもう一度開いてください。',
    notPaidTitle: 'お支払いが見つかりません',
    notPaidBody: 'この購入はまだ支払われていません。請求があった場合は、領収書のメールに返信してください。対応いたします。',
    thanksTitle: 'ありがとうございます。ライセンスキーはこちらです',
    thanksBody: '{email} にライセンスされています。アプリで {unlock} を選び、キーを貼り付けて {activate} を押してください。このページをブックマークしておくと、いつでもキーを確認できます。',
    unlock: 'ロック解除',
    activate: '有効にする',
    copy: 'キーをコピー',
    copied: 'コピーしました',
  },
  ko: {
    incompleteTitle: '링크가 완전하지 않습니다',
    incompleteBody: '결제 확인 메일의 링크를 다시 열어 주세요.',
    notPaidTitle: '결제를 찾을 수 없습니다',
    notPaidBody: '이 구매는 결제되지 않았습니다. 요금이 청구되었다면 영수증 메일에 회신해 주세요. 해결해 드리겠습니다.',
    thanksTitle: '감사합니다! 라이선스 키입니다',
    thanksBody: '{email} 앞으로 라이선스가 발급되었습니다. 앱에서 {unlock}를 선택하고 키를 붙여 넣은 다음 {activate}를 누르세요. 이 페이지를 즐겨찾기에 추가해 두면 언제든지 키를 볼 수 있습니다.',
    unlock: '잠금 해제',
    activate: '활성화',
    copy: '키 복사',
    copied: '복사됨',
  },
  'zh-hans': {
    incompleteTitle: '该链接不完整',
    incompleteBody: '请重新打开付款确认中的链接。',
    notPaidTitle: '未找到付款',
    notPaidBody: '此订单尚未付款。如果已被扣款，请回复收据邮件，我们会为您处理。',
    thanksTitle: '谢谢！这是您的许可证密钥',
    thanksBody: '已授权给 {email}。在应用中选择 {unlock}，粘贴密钥，然后点击 {activate}。请将此页面加入书签：它会一直显示您的密钥。',
    unlock: '解锁',
    activate: '激活',
    copy: '复制密钥',
    copied: '已复制',
  },
  'zh-hant': {
    incompleteTitle: '此連結不完整',
    incompleteBody: '請重新開啟付款確認中的連結。',
    notPaidTitle: '找不到付款',
    notPaidBody: '此訂單尚未付款。如果已被扣款，請回覆收據郵件，我們會為您處理。',
    thanksTitle: '謝謝！這是您的授權金鑰',
    thanksBody: '已授權給 {email}。在應用程式中選擇 {unlock}，貼上金鑰，然後按 {activate}。請將此頁面加入書籤：它會一直顯示您的金鑰。',
    unlock: '解鎖',
    activate: '啟用',
    copy: '複製金鑰',
    copied: '已複製',
  },
};

/** The best supported language tag for an Accept-Language header. */
export function pickLanguage(header: string | undefined): string {
  const wanted = (header ?? '')
    .split(',')
    .map((part) => {
      const [tag, q] = part.trim().split(';q=');
      return { tag: tag.toLowerCase(), q: q === undefined ? 1 : Number(q) || 0 };
    })
    .filter((w) => w.tag)
    .sort((a, b) => b.q - a.q);
  for (const { tag } of wanted) {
    const [language, ...rest] = tag.split('-');
    if (language === 'zh') {
      return rest.some((p) => ['hant', 'tw', 'hk', 'mo'].includes(p)) ? 'zh-hant' : 'zh-hans';
    }
    if (language in texts) return language;
  }
  return 'en';
}
